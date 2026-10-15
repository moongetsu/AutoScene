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

/// Wire types of the protobuf encoding.
const VARINT: u8 = 0;
const I64: u8 = 1;
const LEN: u8 = 2;
const I32: u8 = 5;

// Field numbers (onnx.proto3).
const MODEL_GRAPH: u64 = 7;
const GRAPH_INPUT: u64 = 11;
const GRAPH_OUTPUT: u64 = 12;
const GRAPH_VALUE_INFO: u64 = 13;
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
    }
}
