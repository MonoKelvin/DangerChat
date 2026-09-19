"""M3 推理基座：代理模型生成（设计文档 §8 离线工具链）。

为什么要代理模型：M3 需要「双端耗时」数据来决定设备策略（CPU / DirectML），但真实权重
要到 M4（YOLO 需 dc_uitag 标注后训练、OCR 取官方预训练、语义模型取 HF int8）才到位。
先用**同算量同算子类别**的代理模型把设备交叉点测出来，M4 再用真实模型复测。
代理模型是 M3 期的一次性基准物，不属于发布物（spikes 目录已随 M3 结束移除，
默认输出目录改为 target/bench-models，不再入库）。

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


def yolo_test_stub(path: Path, mode: str, side: int = 320, classes: int = 4) -> None:
    """M4 测试用 YOLO 输出桩（不做卷积，只 Constant → 输出形状）。

    - mode=empty：输出全零 → layout Stage 得到空检出（UT-LAY-04）
    - mode=fixed：输出一个固定框（cx=0.5,cy=0.5,w=0.25,h=0.25,class0,score=0.9）
      → 解码 + letterbox 逆变换的确定性断言

    输出形状 1×(4+classes)×8400 与 v8/v11 导出一致；文件只有几 KB，入库随测试走。
    """
    anchors = 8400
    data = np.zeros((1, 4 + classes, anchors), np.float32)
    if mode == "fixed":
        data[0, 0, 0] = side / 2          # cx
        data[0, 1, 0] = side / 2          # cy
        data[0, 2, 0] = side / 4          # w
        data[0, 3, 0] = side / 4          # h
        data[0, 4, 0] = 0.9               # class0 score
        # 其余锚点 class 分数给 0.05（低于阈值，不产生检出）
        data[0, 4:, 1:] = 0.05

    out = helper.make_tensor_value_info("output", TensorProto.FLOAT, [1, 4 + classes, anchors])
    graph = helper.make_graph(
        [helper.make_node("Constant", [], ["output"], value=numpy_helper.from_array(data, "v"))],
        f"yolo-test-{mode}",
        [helper.make_tensor_value_info("images", TensorProto.FLOAT, [1, 3, side, side])],
        [out],
        [],
    )
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 17)])
    model.ir_version = 10
    onnx.checker.check_model(model)
    onnx.save(model, str(path))


def main() -> int:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "target/bench-models")
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

    # M4 测试桩（小尺寸轻量，入库到 dc-pipeline/tests/fixtures/）
    yolo_test_stub(out / "yolo-test-empty.onnx", "empty")
    print("yolo-test-empty.onnx  恒零输出 → 空检出（UT-LAY-04）")
    yolo_test_stub(out / "yolo-test-fixed.onnx", "fixed")
    print("yolo-test-fixed.onnx  固定框输出 → 解码确定性（UT-LAY-06 伴随）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
