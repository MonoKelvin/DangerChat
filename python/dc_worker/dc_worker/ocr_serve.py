"""M0 保活 OCR：只接收内存 PNG，不读取调用方指定的文件路径。

帧格式：4 字节小端长度 + payload。请求为 PNG（最多 8 MiB），零长度表示退出。
响应最多 1 MiB，首字节 R=就绪、O=成功（随后 UTF-8 文本）、E=失败（固定错误码）。
这是尖峰协议，不是 M1 的 MessagePack 协议。stdout 只输出协议帧。
"""

from __future__ import annotations

import struct
import sys
from contextlib import redirect_stdout
from typing import BinaryIO

from rapidocr_onnxruntime import RapidOCR

MAX_IMAGE_BYTES = 8 * 1024 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"

# 文本检测的长边上限。ROI 已由结构定位裁出，无需按整屏分辨率做检测：
# 736（默认）与 480 在 M0 合成集上精度相同（标题 1.000 / CER 0.0000），
# 但 480 把单次识别的 p50 从约 997 ms 降到约 399 ms。
# 该值只影响检测分辨率，不改变识别模型，若真实界面字号更小需重新标定。
DET_LIMIT_SIDE_LEN = 480


class ProtocolError(ValueError):
    """帧不完整或超过尖峰协议边界。"""


def read_exact(stream: BinaryIO, size: int) -> bytes:
    data = bytearray()
    while len(data) < size:
        chunk = stream.read(size - len(data))
        if not chunk:
            raise ProtocolError("TRUNCATED_FRAME")
        data.extend(chunk)
    return bytes(data)


def read_request(stream: BinaryIO) -> bytes | None:
    first = stream.read(1)
    if not first:
        return None  # 帧间 EOF 可正常退出；半帧 EOF 必须报错。
    size = struct.unpack("<I", first + read_exact(stream, 3))[0]
    if size > MAX_IMAGE_BYTES:
        raise ProtocolError("FRAME_TOO_LARGE")
    if size == 0:
        return None
    return read_exact(stream, size)


def write_response(stream: BinaryIO, payload: bytes) -> None:
    if len(payload) > MAX_RESPONSE_BYTES:
        payload = b"ERESULT_TOO_LARGE"
    stream.write(struct.pack("<I", len(payload)))
    stream.write(payload)
    stream.flush()


def serve_streams(source: BinaryIO, sink: BinaryIO, engine: RapidOCR) -> int:
    write_response(sink, b"R")
    while True:
        try:
            png = read_request(source)
        except ProtocolError:
            write_response(sink, b"EPROTOCOL_ERROR")
            return 1
        if png is None:
            return 0
        if not png.startswith(PNG_SIGNATURE):
            write_response(sink, b"EINVALID_IMAGE")
            continue
        try:
            # 第三方库的 Python stdout 也必须与协议隔离。
            with redirect_stdout(sys.stderr):
                outcome, _ = engine(png)
            text = "".join(str(row[1]) for row in outcome).strip() if outcome else ""
            payload = b"O" + text.encode("utf-8")
        except Exception:
            # 不把图像内容、识别文本或异常中的路径送进诊断日志。
            payload = b"EINFERENCE_FAILED"
        write_response(sink, payload)


def serve() -> int:
    try:
        with redirect_stdout(sys.stderr):
            engine = RapidOCR(
                intra_op_num_threads=2,
                inter_op_num_threads=1,
                det_limit_side_len=DET_LIMIT_SIDE_LEN,
            )
    except Exception:
        write_response(sys.stdout.buffer, b"EINIT_FAILED")
        return 1
    try:
        # 模型推理预热由 Rust 使用有效合成图执行，不以无效 PNG 假装预热成功。
        return serve_streams(sys.stdin.buffer, sys.stdout.buffer, engine)
    except (BrokenPipeError, OSError):
        return 1


if __name__ == "__main__":
    raise SystemExit(serve())
