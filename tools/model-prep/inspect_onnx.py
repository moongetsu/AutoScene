"""Look inside an ONNX graph before the app runs it (dev-only).

    uv run python -I inspect_onnx.py info <model.onnx>
    uv run python -I inspect_onnx.py run <model.onnx> [--size HxW] [--batch N]
    uv run python -I inspect_onnx.py frame <model.onnx> <frame.rgb> <W>x<H>
    uv run python -I inspect_onnx.py free-batch <model.onnx> <out.onnx>

`info` lists inputs and outputs (dtype, fixed or symbolic dims), the opsets,
how many intermediate shapes are declared (and how many of them pin the
batch at 1 — DirectML trusts every declared shape), the ops by count with
the ones DirectML is known to trip on marked, and the weight bytes.

`run` feeds random inputs on the CPU at the given size (default: as
declared) and prints the output shapes; with `--batch N` it frees every
input's batch the way the app does (`as_infer::onnx::reshaped`) and checks
that a batch of N different inputs gives the same outputs as N single runs
(a Reshape with a literal batch of 1 breaks that). A second input named
`scale_factor` (PaddleDetection exports) gets ones.

`free-batch <in> <out>` writes a copy whose constant Reshape targets copy
the batch instead of pinning it at 1 (old PyTorch exports such as
OpenVINO's person ReID), and checks it against the original graph.

`frame` runs one raw RGB24 frame (e.g. `ffmpeg … -vf scale=640:640:flags=
bicubic -f rawvideo -pix_fmt rgb24 frame.rgb`) through a person detector
with the PaddleDetection contract — RGB, (v − mean) / std with ImageNet
statistics in 0..255 — and prints the strongest boxes, to check the app's
own preprocessing against.

Model paths are data: nothing in their folder is imported or executed.
"""

import argparse
import collections
import sys

import numpy as np
import onnx
import onnxruntime as ort
from onnx import numpy_helper

# Ops DirectML rejects or runs on the CPU, or that make shapes data-dependent.
RISKY = {
    "NonZero": "data-dependent shape",
    "NonMaxSuppression": "data-dependent shape",
    "TopK": "k from data when not constant",
    "Loop": "control flow",
    "If": "control flow",
    "Scan": "control flow",
    "GatherND": "slow on DirectML",
    "ScatterND": "slow on DirectML",
    "Range": "shape computed in the graph",
    "ConstantOfShape": "shape computed in the graph",
    "Expand": "shape computed in the graph",
}

DTYPES = {v: k for k, v in onnx.TensorProto.DataType.items()}
NP = {"FLOAT": np.float32, "UINT8": np.uint8, "INT64": np.int64, "INT32": np.int32, "FLOAT16": np.float16}


def dims(t):
    out = []
    for d in t.type.tensor_type.shape.dim:
        out.append(d.dim_value if d.HasField("dim_value") else (d.dim_param or "?"))
    return out


def info(path):
    m = onnx.load(path, load_external_data=False)
    g = m.graph
    print(f"{path}")
    print(f"  ir {m.ir_version}, producer {m.producer_name} {m.producer_version}")
    print("  opsets: " + ", ".join(f"{o.domain or 'ai.onnx'} {o.version}" for o in m.opset_import))
    inits = {i.name for i in g.initializer}
    for kind, values in (("input", [i for i in g.input if i.name not in inits]), ("output", g.output)):
        for v in values:
            dt = DTYPES.get(v.type.tensor_type.elem_type, "?")
            print(f"  {kind:6} {v.name}: {dt} {dims(v)}")
    vi = list(g.value_info)
    batch1 = sum(1 for v in vi if (d := dims(v)) and d[0] == 1)
    print(f"  value_info: {len(vi)} declared, {batch1} with batch 1")
    ops = collections.Counter(n.op_type for n in g.node)
    print(f"  nodes: {sum(ops.values())}")
    for op, n in ops.most_common():
        mark = f"  <-- {RISKY[op]}" if op in RISKY else ""
        print(f"    {op:24} {n:5}{mark}")
    reshape0 = [n.name for n in g.node if n.op_type == "Reshape" and any(a.name == "allowzero" and a.i == 1 for a in n.attribute)]
    if reshape0:
        print(f"  Reshape(allowzero=1): {len(reshape0)} (DirectML rejects them)")
    weight = sum(np.prod(i.dims, dtype=np.int64) * {1: 4, 10: 2, 7: 8, 6: 4, 2: 1, 3: 1}.get(i.data_type, 4) for i in g.initializer)
    print(f"  weights: {weight / 1e6:.1f} MB in {len(g.initializer)} initializers")


def session(path, free_batch=False):
    so = ort.SessionOptions()
    so.log_severity_level = 3
    if not free_batch:
        return ort.InferenceSession(path, so, providers=["CPUExecutionProvider"])
    # What the app does before loading (`as_infer::onnx::reshaped`): no
    # declared intermediate or output shapes, every input's batch free.
    m = onnx.load(path, load_external_data=False)
    del m.graph.value_info[:]
    for o in m.graph.output:
        o.type.tensor_type.ClearField("shape")
    for i in m.graph.input:
        d = i.type.tensor_type.shape.dim
        if d:
            d[0].ClearField("dim_value")
            d[0].dim_param = "N"
    return ort.InferenceSession(m.SerializeToString(), so, providers=["CPUExecutionProvider"])


def feeds(sess, size, batch, rng):
    out = {}
    for i in sess.get_inputs():
        shape = [d if isinstance(d, int) else None for d in i.shape]
        if i.name == "scale_factor":
            out[i.name] = np.ones((batch, 2), np.float32)
            continue
        if len(shape) == 4:
            h, w = size if size else (shape[2] or 640, shape[3] or 640)
            shape = [batch, shape[1] or 3, h, w]
        else:
            shape = [batch if k == 0 else (d or 1) for k, d in enumerate(shape)]
        dt = NP.get(i.type.removeprefix("tensor(").removesuffix(")").upper(), np.float32)
        if dt == np.uint8:
            out[i.name] = rng.integers(0, 256, size=shape, dtype=np.uint8)
        else:
            out[i.name] = rng.standard_normal(shape).astype(dt)
    return out


def run(path, size, batch):
    sess = session(path, free_batch=batch > 1)
    rng = np.random.default_rng(7)
    one = feeds(sess, size, 1, rng)
    names = [o.name for o in sess.get_outputs()]
    res = sess.run(names, one)
    print(f"{path} at {size or 'declared size'}, batch 1:")
    for n, r in zip(names, res):
        print(f"  {n}: {list(r.shape)} {r.dtype} range {float(np.min(r)):.4g}..{float(np.max(r)):.4g}")
    if batch > 1:
        many = feeds(sess, size, batch, rng)
        try:
            together = sess.run(names, many)
        except Exception as e:  # noqa: BLE001 - report and go on
            print(f"  batch {batch}: FAILS ({str(e).splitlines()[0]})")
            return
        apart = [sess.run(names, {k: v[b : b + 1] for k, v in many.items()}) for b in range(batch)]
        for k, n in enumerate(names):
            t = together[k]
            print(f"  batch {batch} {n}: {list(t.shape)}", end="")
            try:
                split = np.concatenate([a[k] for a in apart], axis=0)
                if split.shape == t.shape:
                    print(f", max |batch - single| = {float(np.max(np.abs(split - t))):.3g}")
                else:
                    print(f", single runs concatenate to {list(split.shape)} (layout differs)")
            except ValueError:
                print(", single runs don't concatenate")


def free_batch(path, out):
    """Copy of the graph whose constant Reshape targets copy the batch
    (`0`) instead of pinning it at 1, checked batch-vs-single on the CPU."""
    m = onnx.load(path, load_external_data=False)
    g = m.graph
    init = {i.name: i for i in g.initializer}
    consts = {n.output[0]: n for n in g.node if n.op_type == "Constant"}
    done = set()
    for n in g.node:
        if n.op_type != "Reshape" or n.input[1] in done:
            continue
        if any(a.name == "allowzero" and a.i == 1 for a in n.attribute):
            sys.exit(f"{n.name}: allowzero=1, 0 would mean an empty dimension")
        src = n.input[1]
        if src in init:
            t = init[src]
        elif src in consts:
            t = consts[src].attribute[0].t
        else:
            continue
        v = numpy_helper.to_array(t).copy()
        if v.size and v[0] == 1:
            v[0] = 0
            t.CopyFrom(numpy_helper.from_array(v, t.name))
            done.add(src)
    print(f"{len(done)} Reshape targets now copy the batch")
    onnx.save(m, out)
    rng = np.random.default_rng(3)
    sess = session(out, free_batch=True)
    x = feeds(sess, None, 4, rng)
    names = [o.name for o in sess.get_outputs()]
    together = sess.run(names, x)[0]
    apart = np.concatenate([sess.run(names, {k: v[b : b + 1] for k, v in x.items()})[0] for b in range(4)])
    original = session(path)
    ref = np.concatenate([original.run(names, {k: v[b : b + 1] for k, v in x.items()})[0] for b in range(4)])
    print(f"batch 4 vs single: {float(np.max(np.abs(together - apart))):.3g}; vs the original graph: {float(np.max(np.abs(apart - ref))):.3g}")
    import hashlib

    print(f"{out}: {hashlib.sha256(open(out, 'rb').read()).hexdigest()}")


def frame(path, raw, wh, size):
    w, h = (int(x) for x in wh.split("x"))
    rgb = np.fromfile(raw, np.uint8)
    assert rgb.size == w * h * 3, f"{raw}: {rgb.size} bytes, expected {w}x{h}x3"
    img = rgb.reshape(h, w, 3).astype(np.float32)
    mean = np.array([123.675, 116.28, 103.53], np.float32)
    std = np.array([58.395, 57.12, 57.375], np.float32)
    x = ((img - mean) / std).transpose(2, 0, 1)[None]
    sess = session(path)
    names = [o.name for o in sess.get_outputs()]
    feed = {sess.get_inputs()[0].name: np.ascontiguousarray(x)}
    if any(i.name == "scale_factor" for i in sess.get_inputs()):
        feed["scale_factor"] = np.ones((1, 2), np.float32)
    boxes, scores = sess.run(names, feed)[:2]
    boxes, scores = boxes[0], scores[0, 0]
    order = np.argsort(-scores, kind="stable")
    keep = []
    for i in order:
        if scores[i] < 0.3 or len(keep) >= 20:
            break
        b = boxes[i]
        if all(iou(b, boxes[k]) <= 0.6 for k in keep):
            keep.append(i)
    print(f"{path} on {raw} ({w}x{h}): {len(scores)} candidates, {len(keep)} kept at score >= 0.3")
    for i in keep:
        b = boxes[i]
        print(f"  {scores[i]:.3f}  {b[0]:7.1f} {b[1]:7.1f} {b[2]:7.1f} {b[3]:7.1f}")


def iou(a, b):
    iw = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    ih = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = iw * ih
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / union if union > 0 else 0.0


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("info")
    s.add_argument("model")
    s = sub.add_parser("run")
    s.add_argument("model")
    s.add_argument("--size")
    s.add_argument("--batch", type=int, default=1)
    s = sub.add_parser("frame")
    s.add_argument("model")
    s.add_argument("raw")
    s.add_argument("wh")
    s = sub.add_parser("free-batch")
    s.add_argument("model")
    s.add_argument("out")
    a = p.parse_args()
    size = tuple(int(x) for x in a.size.split("x")) if getattr(a, "size", None) else None
    if a.cmd == "info":
        info(a.model)
    elif a.cmd == "run":
        run(a.model, size, a.batch)
    elif a.cmd == "free-batch":
        free_batch(a.model, a.out)
    else:
        frame(a.model, a.raw, a.wh, size)


if __name__ == "__main__":
    sys.exit(main())
