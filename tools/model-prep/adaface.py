"""AdaFace IR-101 (WebFace12M) → ONNX, for AutoScene's face embedder.

AdaFace (Kim, Jain, Liu; CVPR 2022, arXiv 2204.00964) learns embeddings
whose quality-adaptive margin keeps blurry, small and dark faces close to
their person — the faces ArcFace loses. The weights come from the CVLFace
release on Hugging Face (MIT code; the WebFace12M training data is for
non-commercial research, like InsightFace's models).

The network is defined here, not imported: the upstream repo loads through
`trust_remote_code` and a pickled `model.pt`, and neither is run. Only
`model.safetensors` (plain tensors) is read, pinned by revision and SHA256.

Input: RGB faces aligned to the ArcFace 112×112 five-point template, NCHW,
(v − 127.5) / 127.5 — exactly what AutoScene feeds ArcFace. Output: the
512-d embedding before normalisation (its length says how clear the face
is).

    uv run adaface.py <model.safetensors> <out.onnx>
"""

from __future__ import annotations

import hashlib
import sys
from pathlib import Path

import numpy as np
import onnx
import onnx.numpy_helper
import onnxruntime as ort
import torch
from safetensors.torch import load_file
from torch import nn

REVISION = "54f602a0737bd1ee4a4e7e9fd089a485f397fefd"
SAFETENSORS_SHA256 = "2ea535a43877bd3de8091903935c783ce335be66a9f8917fae9a7a18ae4bbf56"
# Units per stage of IR-100 ("ir101" upstream) and their widths.
STAGES = [(64, 3), (128, 13), (256, 30), (512, 3)]


class Unit(nn.Module):
    """Pre-activation residual unit; the first of a stage halves the size."""

    def __init__(self, cin: int, cout: int, stride: int):
        super().__init__()
        if cin == cout:
            # A 1×1 max pool is plain subsampling.
            self.shortcut_layer = nn.MaxPool2d(1, stride)
        else:
            self.shortcut_layer = nn.Sequential(nn.Conv2d(cin, cout, 1, stride, bias=False), nn.BatchNorm2d(cout))
        self.res_layer = nn.Sequential(
            nn.BatchNorm2d(cin),
            nn.Conv2d(cin, cout, 3, 1, 1, bias=False),
            nn.BatchNorm2d(cout),
            nn.PReLU(cout),
            nn.Conv2d(cout, cout, 3, stride, 1, bias=False),
            nn.BatchNorm2d(cout),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.res_layer(x) + self.shortcut_layer(x)


class IR101(nn.Module):
    def __init__(self):
        super().__init__()
        self.input_layer = nn.Sequential(nn.Conv2d(3, 64, 3, 1, 1, bias=False), nn.BatchNorm2d(64), nn.PReLU(64))
        units, cin = [], 64
        for cout, n in STAGES:
            units.append(Unit(cin, cout, 2))
            units.extend(Unit(cout, cout, 1) for _ in range(n - 1))
            cin = cout
        self.body = nn.Sequential(*units)
        self.output_layer = nn.Sequential(
            nn.BatchNorm2d(512),
            nn.Dropout(0.4),
            nn.Flatten(),
            nn.Linear(512 * 7 * 7, 512),
            nn.BatchNorm1d(512, affine=False),
        )

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.output_layer(self.body(self.input_layer(x)))


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def load(weights: Path) -> IR101:
    got = sha256(weights)
    if got != SAFETENSORS_SHA256:
        sys.exit(f"{weights}: SHA256 {got}, expected {SAFETENSORS_SHA256} (revision {REVISION})")
    state = {k.removeprefix("model.net."): v for k, v in load_file(str(weights)).items()}
    net = IR101()
    # Every tensor has its place and every place its tensor.
    net.load_state_dict(state, strict=True)
    return net.eval()


def for_directml(model: onnx.ModelProto) -> None:
    """DirectML rejects `Reshape(allowzero=1)` ("80070057 Falscher
    Parameter"); with no 0 in the target shape it means the same as the
    default, so it goes."""
    consts = {i.name: onnx.numpy_helper.to_array(i) for i in model.graph.initializer}
    for n in model.graph.node:
        if n.op_type == "Reshape" and any(a.name == "allowzero" for a in n.attribute):
            if n.input[1] not in consts or 0 in consts[n.input[1]].tolist():
                sys.exit(f"{n.name}: Reshape to a shape that may hold 0; allowzero matters")
            keep = [a for a in n.attribute if a.name != "allowzero"]
            del n.attribute[:]
            n.attribute.extend(keep)


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    weights, out = Path(sys.argv[1]), Path(sys.argv[2])
    net = load(weights)
    torch.manual_seed(0)
    example = torch.randn(2, 3, 112, 112)
    with torch.no_grad():
        program = torch.onnx.export(
            net,
            (example,),
            input_names=["input"],
            output_names=["embedding"],
            dynamic_shapes=({0: torch.export.Dim("batch")},),
            dynamo=True,
            optimize=True,
        )
    program.save(str(out), external_data=False)
    model = onnx.load(str(out))
    for_directml(model)
    onnx.checker.check_model(model)
    onnx.save(model, str(out))

    # The graph must give what the network gives, crops of any look.
    x = torch.rand(8, 3, 112, 112) * 2 - 1
    x[4:] *= 0.1  # dull, low-contrast faces too
    with torch.no_grad():
        want = net(x).numpy()
    sess = ort.InferenceSession(str(out), providers=["CPUExecutionProvider"])
    got = sess.run(None, {"input": x.numpy()})[0]
    cos = (want * got).sum(1) / (np.linalg.norm(want, axis=1) * np.linalg.norm(got, axis=1))
    rel = np.abs(np.linalg.norm(got, axis=1) / np.linalg.norm(want, axis=1) - 1)
    print(f"parity: min cosine {cos.min():.7f}, max norm error {rel.max():.2e}")
    if cos.min() < 0.99999 or rel.max() > 1e-4:
        sys.exit("ONNX graph doesn't match the network")
    print(f"{out}: {out.stat().st_size} bytes, sha256 {sha256(out)}")


if __name__ == "__main__":
    main()
