//! Minimal ONNX (protobuf) surgery done in memory at load time, so the model
//! files stay byte-identical to their pinned, SHA256-verified downloads.
//!
//! Exports often declare shapes for the one input size and batch they were
//! traced with (SCRFD: batch 1 on every intermediate value, outputs for
//! 640×640; ArcFace: outputs for batch 1) although the graph computes any.
//! DirectML trusts the declarations and fails on other shapes, so they are
//! dropped (the runtime infers them again), and the input can get the exact
//! shape it will be fed: with every dimension known, DirectML plans the
//! whole graph once instead of operator by operator.
//!
//! Older PyTorch exports also pin the batch inside the graph: their Reshape
//! targets are constants starting with 1 (OpenVINO's person ReID has 27).
//! [`free_reshape_batch`] makes them copy the batch (0) instead, in place.

/// Wire types of the protobuf encoding.
const VARINT: u8 = 0;
const I64: u8 = 1;
const LEN: u8 = 2;
const I32: u8 = 5;

// Field numbers (onnx.proto3).
const MODEL_GRAPH: u64 = 7;
const GRAPH_NODE: u64 = 1;
const GRAPH_INITIALIZER: u64 = 5;
const GRAPH_INPUT: u64 = 11;
const GRAPH_OUTPUT: u64 = 12;
const GRAPH_VALUE_INFO: u64 = 13;
const NODE_INPUT: u64 = 1;
const NODE_OUTPUT: u64 = 2;
const NODE_OP_TYPE: u64 = 4;
const NODE_ATTRIBUTE: u64 = 5;
const ATTRIBUTE_NAME: u64 = 1;
const ATTRIBUTE_I: u64 = 3;
const ATTRIBUTE_T: u64 = 5;
const TENSOR_DATA_TYPE: u64 = 2;
const TENSOR_INT64_DATA: u64 = 7;
const TENSOR_NAME: u64 = 8;
const TENSOR_RAW_DATA: u64 = 9;
/// `TensorProto.DataType.INT64`.
const INT64: u64 = 7;
const VALUE_INFO_TYPE: u64 = 2;
const TYPE_TENSOR: u64 = 1;
const TENSOR_SHAPE: u64 = 2;
const SHAPE_DIM: u64 = 1;
const DIM_VALUE: u64 = 1;
const DIM_PARAM: u64 = 2;

fn varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// One field of a message: (number, wire type, raw bytes of the whole field,
/// payload range for length-delimited fields).
struct Field<'a> {
    number: u64,
    wire: u8,
    raw: &'a [u8],
    payload: &'a [u8],
}

fn fields(msg: &[u8]) -> Option<Vec<Field<'_>>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < msg.len() {
        let start = pos;
        let key = varint(msg, &mut pos)?;
        let (number, wire) = (key >> 3, (key & 7) as u8);
        let payload_start;
        match wire {
            VARINT => {
                varint(msg, &mut pos)?;
                payload_start = pos;
            }
            I64 => {
                pos += 8;
                payload_start = pos;
            }
            I32 => {
                pos += 4;
                payload_start = pos;
            }
            LEN => {
                let len = usize::try_from(varint(msg, &mut pos)?).ok()?;
                payload_start = pos;
                pos = pos.checked_add(len)?;
            }
            _ => return None,
        }
        if pos > msg.len() {
            return None;
        }
        let payload = if wire == LEN { &msg[payload_start..pos] } else { &msg[pos..pos] };
        out.push(Field { number, wire, raw: &msg[start..pos], payload });
    }
    Some(out)
}

/// Re-encode `msg`, replacing the payload of length-delimited fields for
/// which `edit` returns `Some` (an empty payload `Some(None)` drops the field).
fn rewrite(msg: &[u8], mut edit: impl FnMut(u64, &[u8]) -> Option<Option<Vec<u8>>>) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(msg.len());
    for f in fields(msg)? {
        let replaced = if f.wire == LEN { edit(f.number, f.payload) } else { None };
        match replaced {
            None => out.extend_from_slice(f.raw),
            Some(None) => {}
            Some(Some(p)) => {
                put_varint(&mut out, (f.number << 3) | u64::from(LEN));
                put_varint(&mut out, p.len() as u64);
                out.extend_from_slice(&p);
            }
        }
    }
    Some(out)
}

/// How [`reshaped`] treats the shape of the graph's first input.
#[derive(Clone, Copy, Debug)]
pub enum InputShape<'a> {
    /// As declared.
    Declared,
    /// A first dimension fixed at 1 becomes the free batch `N`.
    FreeBatch,
    /// Exactly these dimensions (the rank must match the declared one).
    Fixed(&'a [usize]),
}

/// The model without the shapes it declares for its outputs and intermediate
/// values, its first input shaped as asked. `None` if the bytes aren't a
/// readable ONNX model or a fixed shape doesn't fit the declared rank.
pub fn reshaped(model: &[u8], input: InputShape) -> Option<Vec<u8>> {
    let mut ok = true;
    let mut first_input = true;
    let out = rewrite(model, |n, graph| {
        (n == MODEL_GRAPH).then(|| {
            let g = rewrite(graph, |n, value_info| match n {
                GRAPH_VALUE_INFO => Some(None),
                GRAPH_OUTPUT => {
                    let v = rewrite_shape(value_info, |_| Some(None));
                    ok &= v.is_some();
                    Some(v)
                }
                GRAPH_INPUT if std::mem::take(&mut first_input) => {
                    let v = match input {
                        InputShape::Declared => return None,
                        InputShape::FreeBatch => rewrite_shape(value_info, |shape| free_batch(shape).map(Some)),
                        InputShape::Fixed(dims) => {
                            let mut found = false;
                            let v = rewrite_shape(value_info, |shape| {
                                found = true;
                                fixed(shape, dims).map(Some)
                            });
                            v.filter(|_| found)
                        }
                    };
                    ok &= v.is_some();
                    Some(v)
                }
                _ => None,
            });
            ok &= g.is_some();
            g
        })
    })?;
    ok.then_some(out)
}

/// Re-encode a `ValueInfoProto`, replacing its tensor shape with what `f`
/// returns for it (`Some(None)` drops it; `None` fails the rewrite).
fn rewrite_shape(value_info: &[u8], mut f: impl FnMut(&[u8]) -> Option<Option<Vec<u8>>>) -> Option<Vec<u8>> {
    let ok = std::cell::Cell::new(true);
    let fail_unless = |done: bool| ok.set(ok.get() && done);
    let out = rewrite(value_info, |n, ty| {
        (n == VALUE_INFO_TYPE).then(|| {
            let t = rewrite(ty, |n, tensor| {
                (n == TYPE_TENSOR).then(|| {
                    let s = rewrite(tensor, |n, shape| {
                        (n == TENSOR_SHAPE).then(|| {
                            let r = f(shape);
                            fail_unless(r.is_some());
                            r.flatten()
                        })
                    });
                    fail_unless(s.is_some());
                    s
                })
            });
            fail_unless(t.is_some());
            t
        })
    })?;
    ok.get().then_some(out)
}

/// A `TensorShapeProto` whose first dimension, if it is the fixed value 1,
/// becomes the symbol `N`.
fn free_batch(shape: &[u8]) -> Option<Vec<u8>> {
    let mut first = true;
    rewrite(shape, |n, dim| {
        if n != SHAPE_DIM || !std::mem::take(&mut first) {
            return None;
        }
        let fixed_one =
            fields(dim)?.iter().any(|f| f.number == DIM_VALUE && f.wire == VARINT && dim_value(f) == Some(1));
        fixed_one.then(|| {
            let mut d = Vec::new();
            put_len_field(&mut d, DIM_PARAM, b"N");
            Some(d)
        })
    })
}

/// A `TensorShapeProto` of exactly `dims`, if `shape` has that many.
fn fixed(shape: &[u8], dims: &[usize]) -> Option<Vec<u8>> {
    let rank = fields(shape)?.iter().filter(|f| f.number == SHAPE_DIM).count();
    if rank != dims.len() {
        return None;
    }
    let mut out = Vec::new();
    for &d in dims {
        let mut dim = Vec::new();
        put_varint(&mut dim, DIM_VALUE << 3);
        put_varint(&mut dim, d as u64);
        put_len_field(&mut out, SHAPE_DIM, &dim);
    }
    Some(out)
}

fn dim_value(f: &Field) -> Option<u64> {
    let mut p = 0;
    varint(f.raw, &mut p)?;
    varint(f.raw, &mut p)
}

fn put_len_field(out: &mut Vec<u8>, number: u64, payload: &[u8]) {
    put_varint(out, (number << 3) | u64::from(LEN));
    put_varint(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

/// Where `part` (a slice of `whole`) starts in it.
fn offset_in(whole: &[u8], part: &[u8]) -> usize {
    part.as_ptr() as usize - whole.as_ptr() as usize
}

fn varint_value(f: &Field) -> Option<u64> {
    if f.wire != VARINT {
        return None;
    }
    let mut p = 0;
    varint(f.raw, &mut p)?;
    varint(f.raw, &mut p)
}

/// Where in `model` the first value of an INT64 `TensorProto` (`tensor`, a
/// slice of `model`) is stored, if that value is exactly 1: the byte that
/// turns it into 0 (raw data is little-endian; varints of 0 and 1 are one byte).
fn first_one(model: &[u8], tensor: &[u8]) -> Option<Option<usize>> {
    let fs = fields(tensor)?;
    if !fs.iter().any(|f| f.number == TENSOR_DATA_TYPE && varint_value(f) == Some(INT64)) {
        return Some(None);
    }
    for f in &fs {
        match (f.number, f.wire) {
            (TENSOR_RAW_DATA, LEN) => {
                let v = f.payload.get(..8).map(|b| i64::from_le_bytes(b.try_into().expect("8 bytes")));
                return Some((v == Some(1)).then(|| offset_in(model, f.payload)));
            }
            // Packed: the payload is the varints.
            (TENSOR_INT64_DATA, LEN) => {
                return Some((f.payload.first() == Some(&1)).then(|| offset_in(model, f.payload)));
            }
            // Unpacked: a field per value; a value of 1 is the field's last byte.
            (TENSOR_INT64_DATA, VARINT) => {
                let last = &f.raw[f.raw.len() - 1..];
                return Some((varint_value(f) == Some(1)).then(|| offset_in(model, last)));
            }
            _ => {}
        }
    }
    Some(None)
}

/// The model with every constant Reshape target that pins the batch at 1
/// copying it (0) instead — what `tools/model-prep/inspect_onnx.py
/// free-batch` writes to a file, done in memory: only those bytes change.
/// Returns it and how many targets changed; `None` if the bytes aren't a
/// readable ONNX model, or a Reshape takes 0 literally (`allowzero`).
pub fn free_reshape_batch(model: &[u8]) -> Option<(Vec<u8>, usize)> {
    let graph = fields(model)?.into_iter().find(|f| f.number == MODEL_GRAPH && f.wire == LEN)?.payload;
    let strings = |fs: &[Field], n: u64| -> Vec<Vec<u8>> {
        fs.iter().filter(|f| f.number == n && f.wire == LEN).map(|f| f.payload.to_vec()).collect()
    };
    // Tensors by name: initializers, and the values of Constant nodes.
    let mut tensors: std::collections::HashMap<Vec<u8>, &[u8]> = std::collections::HashMap::new();
    let mut targets: Vec<Vec<u8>> = Vec::new();
    for f in fields(graph)? {
        match (f.number, f.wire) {
            (GRAPH_INITIALIZER, LEN) => {
                if let Some(name) = strings(&fields(f.payload)?, TENSOR_NAME).pop() {
                    tensors.insert(name, f.payload);
                }
            }
            (GRAPH_NODE, LEN) => {
                let node = fields(f.payload)?;
                let op = strings(&node, NODE_OP_TYPE).pop().unwrap_or_default();
                let attributes: Vec<Vec<Field>> = node
                    .iter()
                    .filter(|a| a.number == NODE_ATTRIBUTE && a.wire == LEN)
                    .map(|a| fields(a.payload))
                    .collect::<Option<_>>()?;
                let named = |name: &[u8]| {
                    attributes.iter().find(|a| strings(a, ATTRIBUTE_NAME).first().map(Vec::as_slice) == Some(name))
                };
                match op.as_slice() {
                    b"Reshape" => {
                        let allowzero = named(b"allowzero")
                            .and_then(|a| a.iter().find(|x| x.number == ATTRIBUTE_I).and_then(varint_value));
                        if allowzero == Some(1) {
                            return None;
                        }
                        targets.extend(strings(&node, NODE_INPUT).into_iter().nth(1));
                    }
                    b"Constant" => {
                        let value =
                            named(b"value").and_then(|a| a.iter().find(|x| x.number == ATTRIBUTE_T && x.wire == LEN));
                        if let (Some(out), Some(t)) = (strings(&node, NODE_OUTPUT).pop(), value) {
                            tensors.insert(out, t.payload);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    targets.sort();
    targets.dedup();
    let mut at: Vec<usize> = Vec::new();
    for t in &targets {
        if let Some(tensor) = tensors.get(t)
            && let Some(byte) = first_one(model, tensor)?
        {
            at.push(byte);
        }
    }
    let mut out = model.to_vec();
    for &byte in &at {
        out[byte] = 0;
    }
    Some((out, at.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len_field(out: &mut Vec<u8>, number: u64, payload: &[u8]) {
        put_varint(out, (number << 3) | u64::from(LEN));
        put_varint(out, payload.len() as u64);
        out.extend_from_slice(payload);
    }

    fn varint_field(out: &mut Vec<u8>, number: u64, v: u64) {
        put_varint(out, number << 3);
        put_varint(out, v);
    }

    /// A dimension: fixed value or symbol.
    enum D {
        V(u64),
        P(&'static str),
    }

    /// ValueInfoProto { name, type { tensor_type { elem_type: FLOAT, shape? } } }.
    fn value_info(name: &str, dims: Option<&[D]>) -> Vec<u8> {
        let mut tensor = Vec::new();
        varint_field(&mut tensor, 1, 1);
        if let Some(dims) = dims {
            let mut shape = Vec::new();
            for d in dims {
                let mut dim = Vec::new();
                match d {
                    D::V(v) => varint_field(&mut dim, DIM_VALUE, *v),
                    D::P(p) => len_field(&mut dim, DIM_PARAM, p.as_bytes()),
                }
                len_field(&mut shape, SHAPE_DIM, &dim);
            }
            len_field(&mut tensor, TENSOR_SHAPE, &shape);
        }
        let mut ty = Vec::new();
        len_field(&mut ty, TYPE_TENSOR, &tensor);
        let mut vi = Vec::new();
        len_field(&mut vi, 1, name.as_bytes());
        len_field(&mut vi, VALUE_INFO_TYPE, &ty);
        vi
    }

    /// ModelProto { ir_version: 6, graph { fields } }.
    fn model(graph: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut g = Vec::new();
        for (number, payload) in graph {
            len_field(&mut g, *number, payload);
        }
        let mut m = Vec::new();
        varint_field(&mut m, 1, 6);
        len_field(&mut m, MODEL_GRAPH, &g);
        m
    }

    /// A node longer than 127 bytes (two-byte length prefix).
    fn node() -> (u64, Vec<u8>) {
        (1, vec![0xAA; 300])
    }

    fn input(dims: &[D]) -> (u64, Vec<u8>) {
        (GRAPH_INPUT, value_info("input.1", Some(dims)))
    }

    use D::{P, V};

    #[test]
    fn declared_shapes_go_and_nothing_else_changes() {
        let x = input(&[V(1), V(3), P("?"), P("?")]);
        let original = model(&[
            node(),
            x.clone(),
            (GRAPH_VALUE_INFO, value_info("277", Some(&[V(1), V(28), P("?"), P("?")]))),
            (GRAPH_OUTPUT, value_info("683", Some(&[V(1), V(512)]))),
        ]);
        let expected = model(&[node(), x, (GRAPH_OUTPUT, value_info("683", None))]);
        assert_eq!(reshaped(&original, InputShape::Declared).unwrap(), expected);
        assert_eq!(reshaped(&expected, InputShape::Declared).unwrap(), expected);
    }

    #[test]
    fn a_batch_fixed_at_one_becomes_free() {
        let m = model(&[node(), input(&[V(1), V(3), V(640), V(640)])]);
        let free = model(&[node(), input(&[P("N"), V(3), V(640), V(640)])]);
        assert_eq!(reshaped(&m, InputShape::FreeBatch).unwrap(), free);
        // A batch that isn't 1 stays as it is.
        let m = model(&[input(&[V(4), V(3)])]);
        assert_eq!(reshaped(&m, InputShape::FreeBatch).unwrap(), m);
    }

    #[test]
    fn a_fixed_shape_replaces_every_dimension_of_the_first_input() {
        let second = (GRAPH_INPUT, value_info("mask", Some(&[V(1), P("?")])));
        let m = model(&[node(), input(&[V(1), V(3), P("?"), P("?")]), second.clone()]);
        let fixed = model(&[node(), input(&[V(4), V(3), V(384), V(640)]), second]);
        assert_eq!(reshaped(&m, InputShape::Fixed(&[4, 3, 384, 640])).unwrap(), fixed);
        // The rank must match, and there must be a shape to replace.
        assert!(reshaped(&m, InputShape::Fixed(&[4, 3])).is_none());
        let unshaped = model(&[(GRAPH_INPUT, value_info("x", None))]);
        assert!(reshaped(&unshaped, InputShape::Fixed(&[1])).is_none());
    }

    #[test]
    fn garbage_is_refused() {
        assert!(reshaped(&[0x3A, 0xFF, 0xFF], InputShape::Declared).is_none());
        assert!(free_reshape_batch(&[0x3A, 0xFF, 0xFF]).is_none());
    }

    /// An INT64 TensorProto named `name` holding `values` as raw data,
    /// packed varints or one varint field each.
    fn int64s(name: &str, values: &[i64], how: u8) -> Vec<u8> {
        let mut t = Vec::new();
        varint_field(&mut t, 1, values.len() as u64);
        varint_field(&mut t, TENSOR_DATA_TYPE, INT64);
        len_field(&mut t, TENSOR_NAME, name.as_bytes());
        match how {
            b'r' => {
                len_field(&mut t, TENSOR_RAW_DATA, &values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
            }
            b'p' => {
                let mut packed = Vec::new();
                values.iter().for_each(|&v| put_varint(&mut packed, v as u64));
                len_field(&mut t, TENSOR_INT64_DATA, &packed);
            }
            _ => values.iter().for_each(|&v| varint_field(&mut t, TENSOR_INT64_DATA, v as u64)),
        }
        t
    }

    fn reshape(input: &str, shape: &str, allowzero: Option<u64>) -> (u64, Vec<u8>) {
        let mut n = Vec::new();
        len_field(&mut n, NODE_INPUT, input.as_bytes());
        len_field(&mut n, NODE_INPUT, shape.as_bytes());
        len_field(&mut n, NODE_OUTPUT, format!("{input}_r").as_bytes());
        len_field(&mut n, NODE_OP_TYPE, b"Reshape");
        if let Some(z) = allowzero {
            let mut a = Vec::new();
            len_field(&mut a, ATTRIBUTE_NAME, b"allowzero");
            varint_field(&mut a, ATTRIBUTE_I, z);
            len_field(&mut n, NODE_ATTRIBUTE, &a);
        }
        (GRAPH_NODE, n)
    }

    fn constant(output: &str, tensor: Vec<u8>) -> (u64, Vec<u8>) {
        let mut n = Vec::new();
        len_field(&mut n, NODE_OUTPUT, output.as_bytes());
        len_field(&mut n, NODE_OP_TYPE, b"Constant");
        let mut a = Vec::new();
        len_field(&mut a, ATTRIBUTE_NAME, b"value");
        len_field(&mut a, ATTRIBUTE_T, &tensor);
        len_field(&mut n, NODE_ATTRIBUTE, &a);
        (GRAPH_NODE, n)
    }

    #[test]
    fn reshapes_that_pin_the_batch_copy_it_instead() {
        // A float weight starting like a 1 must stay as it is.
        let mut weight = Vec::new();
        varint_field(&mut weight, TENSOR_DATA_TYPE, 1);
        len_field(&mut weight, TENSOR_NAME, b"w");
        len_field(&mut weight, TENSOR_RAW_DATA, &[1, 0, 0, 0, 0, 0, 0, 0]);
        let graph = |third: (u64, Vec<u8>)| {
            model(&[
                (GRAPH_INITIALIZER, int64s("raw", &[1, -1], b'r')),
                (GRAPH_INITIALIZER, int64s("packed", &[1, 256, 8, 16], b'p')),
                (GRAPH_INITIALIZER, int64s("batched", &[4, -1], b'r')),
                (GRAPH_INITIALIZER, int64s("big", &[129, 2], b'u')),
                (GRAPH_INITIALIZER, weight.clone()),
                constant("constant", int64s("", &[1, 64], b'u')),
                reshape("a", "raw", None),
                reshape("b", "packed", Some(0)),
                reshape("c", "constant", None),
                reshape("d", "batched", None),
                reshape("e", "big", None),
                reshape("f", "raw", None),
                reshape("g", "w", None),
                third,
            ])
        };
        // A target computed in the graph (no constant of that name) is left alone.
        let m = graph(reshape("x", "computed", None));
        let (free, n) = free_reshape_batch(&m).unwrap();
        assert_eq!(n, 3, "raw, packed and the constant; a batch of 4, 129 and a float stay");
        assert_eq!(free.len(), m.len());
        let changed: Vec<usize> = (0..m.len()).filter(|&i| m[i] != free[i]).collect();
        assert_eq!(changed.len(), 3);
        assert!(changed.iter().all(|&i| m[i] == 1 && free[i] == 0));
        // Done once, there is nothing left to do.
        assert_eq!(free_reshape_batch(&free).unwrap().1, 0);
        // A Reshape that takes 0 literally can't be freed this way.
        assert!(free_reshape_batch(&graph(reshape("h", "raw", Some(1)))).is_none());
    }
}
