# model-prep

Dev-only tools that turn upstream face models into the ONNX graphs
AutoScene runs. Nothing here ships: the app is 100 % Rust and only ever
loads the finished `.onnx` files.

Python is managed by [uv](https://docs.astral.sh/uv/); dependencies are the
newest releases (pre-releases included), pinned exactly in `pyproject.toml`.

## AdaFace IR-101 (WebFace12M)

`adaface.py` exports the identity embedder AdaFace (Kim, Jain, Liu, CVPR
2022) from the CVLFace release on Hugging Face
(`minchul/cvlface_adaface_ir101_webface12m`, revision `54f602a`):

```sh
curl -L -o model.safetensors https://huggingface.co/minchul/cvlface_adaface_ir101_webface12m/resolve/54f602a0737bd1ee4a4e7e9fd089a485f397fefd/model.safetensors
PYTHONIOENCODING=utf-8 uv run adaface.py model.safetensors adaface_ir101_webface12m.onnx
```

- The network is defined in `adaface.py`; the upstream repo's code
  (`trust_remote_code`) and its pickled `model.pt` are never run — only
  the plain tensors of `model.safetensors`, checked against their SHA256.
- Input and alignment are exactly ArcFace's (RGB, 112×112 five-point
  template, `(v − 127.5) / 127.5`), so the app's embedder runs either graph.
- The export is checked against the PyTorch network (cosine ≥ 0.99999 on
  random and low-contrast crops) and made loadable by DirectML
  (`Reshape(allowzero=1)`, which DirectML rejects, is dropped where it
  means nothing).
- Licence: code MIT; the WebFace12M training data is for non-commercial
  research — fine for AutoScene's free, personal use, like InsightFace's
  models.

Result on the identity bench (`bench/identity`): AdaFace IR-101 separates
the main cast less well than InsightFace glintr100 (per-track F 0.887 vs
0.925), so the app ships glintr100. The exporter stays for later models.

## Looking inside a graph

`inspect_onnx.py` checks an upstream ONNX file before the app loads it
(paths are data; run it with `python -I`):

```sh
uv run python -I inspect_onnx.py info <model.onnx>            # inputs, batch-1 shapes, risky ops
uv run python -I inspect_onnx.py run <model.onnx> --batch 4   # does a batch give what single runs give?
uv run python -I inspect_onnx.py free-batch <in.onnx> <out.onnx>
```

`free-batch` makes old PyTorch exports that pin the batch at 1 in their
Reshapes (OpenVINO's person ReID, `bench/bodies`) run in batches, checked
against the original graph.
