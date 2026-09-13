"""M3 推理基座：代理模型生成（设计文档 §8 离线工具链）。

为什么要代理模型：M3 需要「双端耗时」数据来决定设备策略（CPU / DirectML），但真实权重
要到 M4（YOLO 需 dc_uitag 标注后训练、OCR 取官方预训练、语义模型取 HF int8）才到位。
先用**同算量同算子类别**的代理模型把设备交叉点测出来，M4 再用真实模型复测。
代理模型只进 spikes/，不属于发布物。

用法：
    python tools/training/make_bench_models.py <out_dir>
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import onnx
from onnx import TensorProto, helper, numpy_helper


def conv_chain(
    path: Path,
    input_shape: tuple[int, int, int, int],
    layers: list[tuple[int, int, int]],
    num_classes: int = 10,
) -> float:
    """构建 conv+relu 主干 + 全局池化 + Gemm 头部，返回估算 GFLOPs。"""
    nodes: list[onnx.NodeProto] = []
    inits: list[onnx.TensorProto] = []
    prev = "input"
    prev_c = input_shape[1]
    h, w = input_shape[2], input_shape[3]
    gflops = 0.0

    for i, (cin, cout, stride) in enumerate(layers):
        wgt = numpy_helper.from_array(
            (np.random.default_rng(i).standard_normal((cout, cin, 3, 3)) * 0.05).astype(np.float32),
            f"w{i}",
        )
        bias = numpy_helper.from_array(np.zeros(cout, np.float32), f"b{i}")
        inits += [wgt, bias]
        nodes.append(
            helper.make_node(
                "Conv",
                [prev, f"w{i}", f"b{i}"],
                [f"c{i}"],
                kernel_shape=[3, 3],
                pads=[1, 1, 1, 1],
                strides=[stride, stride],
            )
        )
        nodes.append(helper.make_node("Relu", [f"c{i}"], [f"r{i}"]))
        prev = f"r{i}"
        out_h = (h + 2 - 3) // stride + 1
        out_w = (w + 2 - 3) // stride + 1
        gflops += 2.0 * cin * cout * 9 * out_h * out_w / 1e9
        h, w = out_h, out_w
        prev_c = cout

    nodes.append(helper.make_node("GlobalAveragePool", [prev], ["gap"]))
    nodes.append(helper.make_node("Flatten", ["gap"], ["flat"], axis=1))
    inits.append(
        numpy_helper.from_array(
            (np.random.default_rng(99).standard_normal((prev_c, num_classes)) * 0.05).astype(
                np.float32
            ),
            "hw",
        )
    )
    inits.append(numpy_helper.from_array(np.zeros(num_classes, np.float32), "hb"))
    nodes.append(helper.make_node("Gemm", ["flat", "hw", "hb"], ["output"]))

    graph = helper.make_graph(
        nodes,
        "proxy",
        [helper.make_tensor_value_info("input", TensorProto.FLOAT, list(input_shape))],
        [helper.make_tensor_value_info("output", TensorProto.FLOAT, [input_shape[0], num_classes])],
        inits,
    )
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 17)])
    model.ir_version = 10  # 兼容 ORT 1.28 的 protobuf 解析
    onnx.checker.check_model(model)
    onnx.save(model, str(path))
    return gflops


def matmul_chain(path: Path, tokens: int = 512, dim: int = 384, layers: int = 6) -> float:
    """构建「句向量模型」代理：Gather 嵌入 + N 层 MatMul/Add/Relu + ReduceMean。"""
    nodes: list[onnx.NodeProto] = []
    inits: list[onnx.TensorProto] = []

    inits.append(
        numpy_helper.from_array(
            (np.random.default_rng(1).standard_normal((30522, dim)) * 0.02).astype(np.float32),
            "embed",
        )
    )
    nodes.append(helper.make_node("Gather", ["embed", "input"], ["h0"], axis=0))
    flops = 0.0
    cur = "h0"
    for i in range(layers):
        inits.append(
            numpy_helper.from_array(
                (np.random.default_rng(i + 10).standard_normal((dim, dim)) * 0.02).astype(np.float32),
                f"w{i}",
            )
        )
        inits.append(numpy_helper.from_array(np.zeros(dim, np.float32), f"b{i}"))
        nodes.append(helper.make_node("MatMul", [cur, f"w{i}"], [f"mm{i}"]))
        nodes.append(helper.make_node("Add", [f"mm{i}", f"b{i}"], [f"ad{i}"]))
        nodes.append(helper.make_node("Relu", [f"ad{i}"], [f"h{i + 1}"]))
        cur = f"h{i + 1}"
        flops += 2.0 * tokens * dim * dim / 1e9
    nodes.append(helper.make_node("ReduceMean", [cur], ["output"], axes=[1], keepdims=0))

    graph = helper.make_graph(
        nodes,
        "sem-proxy",
        [helper.make_tensor_value_info("input", TensorProto.INT64, [1, tokens])],
        [helper.make_tensor_value_info("output", TensorProto.FLOAT, [1, dim])],
        inits,
    )
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 17)])
    model.ir_version = 10
    onnx.checker.check_model(model)
    onnx.save(model, str(path))
    return flops


def main() -> int:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "spikes/m3-inference/models")
    out.mkdir(parents=True, exist_ok=True)

    # YOLOv8n@640 约 8.7 GFLOPs：下面这条链约 10.6 GFLOPs，量级一致
    yolo = [
        (3, 32, 2),
        (32, 64, 2),
        (64, 128, 2),
        (128, 128, 1),
        (128, 128, 1),
        (128, 256, 2),
        (256, 256, 1),
        (256, 256, 1),
    ]
    g = conv_chain(out / "yolo-proxy-640.onnx", (1, 3, 640, 640), yolo)
    print(f"yolo-proxy-640.onnx   输入 1x3x640x640  约 {g:.2f} GFLOPs（YOLOv8n@640 约 8.7）")

    # OCR det（960x960）与 rec（3x48x320）代理
    det = [(3, 32, 2), (32, 64, 2), (64, 128, 2), (128, 128, 1), (128, 256, 2), (256, 256, 1)]
    g = conv_chain(out / "ocr-det-proxy-960.onnx", (1, 3, 960, 960), det)
    print(f"ocr-det-proxy-960.onnx 输入 1x3x960x960 约 {g:.2f} GFLOPs")
    g = conv_chain(
        out / "ocr-rec-proxy.onnx",
        (1, 3, 48, 320),
        [(3, 32, 1), (32, 64, 1), (64, 128, 1), (128, 128, 1)],
    )
    print(f"ocr-rec-proxy.onnx    输入 1x3x48x320  约 {g:.2f} GFLOPs")

    g = matmul_chain(out / "sem-proxy-int64.onnx")
    print(f"sem-proxy-int64.onnx  输入 1x512(i64)   约 {g:.2f} GFLOPs（bge-small 级 6 层代理）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
