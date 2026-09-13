"""尖峰帧协议测试：只用假引擎，不加载模型。"""

from __future__ import annotations

import io
import struct

import pytest

from dc_worker.ocr_serve import (
    MAX_IMAGE_BYTES,
    MAX_RESPONSE_BYTES,
    PNG_SIGNATURE,
    ProtocolError,
    read_request,
    serve_streams,
    write_response,
)


def frame(payload: bytes) -> bytes:
    return struct.pack("<I", len(payload)) + payload


def responses(sink: io.BytesIO) -> list[bytes]:
    source = io.BytesIO(sink.getvalue())
    result = []
    while payload := read_request(source):
        result.append(payload)
    return result


class ShortReads(io.BytesIO):
    def read(self, size: int = -1) -> bytes:
        return super().read(min(size, 2))


def test_partial_pipe_reads_are_reassembled():
    assert read_request(ShortReads(frame(PNG_SIGNATURE))) == PNG_SIGNATURE


@pytest.mark.parametrize("data", [b"\x01", b"\x01\x00\x00", frame(b"png")[:-1]])
def test_truncated_frame_is_not_clean_eof(data: bytes):
    with pytest.raises(ProtocolError):
        read_request(io.BytesIO(data))


def test_oversized_length_is_rejected_before_reading_body():
    with pytest.raises(ProtocolError, match="FRAME_TOO_LARGE"):
        read_request(io.BytesIO(struct.pack("<I", MAX_IMAGE_BYTES + 1)))


@pytest.mark.parametrize("end", [b"", frame(b"")])
def test_clean_shutdown(end: bytes):
    sink = io.BytesIO()
    assert serve_streams(io.BytesIO(end), sink, lambda _: pytest.fail("unexpected inference")) == 0
    assert responses(sink) == [b"R"]


def test_multiline_unicode_and_old_error_prefix_are_normal_text(capsys):
    def engine(png: bytes):
        assert png == PNG_SIGNATURE
        print("third party log")
        return [[[], "__ERROR__ 合成\n文字", 1.0]], None

    sink = io.BytesIO()
    assert serve_streams(io.BytesIO(frame(PNG_SIGNATURE) * 2), sink, engine) == 0
    assert responses(sink) == [b"R"] + ["O__ERROR__ 合成\n文字".encode()] * 2
    captured = capsys.readouterr()
    assert captured.out == ""
    assert "third party log" in captured.err


def test_empty_recognition_is_distinct_from_inference_failure():
    calls = iter([None, RuntimeError("private content")])

    def engine(_png: bytes):
        result = next(calls)
        if isinstance(result, Exception):
            raise result
        return result, None

    sink = io.BytesIO()
    assert serve_streams(io.BytesIO(frame(PNG_SIGNATURE) * 2), sink, engine) == 0
    assert responses(sink) == [b"R", b"O", b"EINFERENCE_FAILED"]


def test_file_path_is_not_opened_or_sent_to_engine():
    sink = io.BytesIO()
    assert serve_streams(
        io.BytesIO(frame(b"C:/private.png")), sink, lambda _: pytest.fail("unexpected inference")
    ) == 0
    assert responses(sink) == [b"R", b"EINVALID_IMAGE"]


def test_malformed_frame_terminates_session():
    sink = io.BytesIO()
    assert serve_streams(io.BytesIO(b"\x01"), sink, lambda _: None) == 1
    assert responses(sink) == [b"R", b"EPROTOCOL_ERROR"]


def test_oversized_result_is_an_error_frame():
    sink = io.BytesIO()
    write_response(sink, b"O" * (MAX_RESPONSE_BYTES + 1))
    assert responses(sink) == [b"ERESULT_TOO_LARGE"]
