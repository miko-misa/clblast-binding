# clblast-binding (alpha)

Safe-ish bindings for [CLBlast](https://github.com/CNugteren/CLBlast) and auto-generated wrappers that integrate with [`ocl`](https://crates.io/crates/ocl) queues, buffers and events.

> **Status:** alpha. APIs may change.

## Features

- ✅ Provides static binding with [bindgen](https://github.com/rust-lang/rust-bindgen) and wrappers using the [ocl](https://github.com/cogciprocate/ocl) for the included CLBlast and OpenCL Header.
- \[in progress\]Provides an option to perform binding using the user's local CLBlast.

## Install

```toml
[dependencies]
clblast-binding = { git = "https://github.com/miko-misa/clblast-binding" }
```

## Quick Start

```rust
use ocl::{Buffer, ProQue};
use clblast_binding::{sgemm, clblast_sys::CLBlastLayout, clblast_sys::CLBlastTranspose};

fn main() -> ocl::Result<()> {
  let m = 2; let n = 2; let k = 2;
  let pq = ProQue::builder().src("__kernel void nop(){}").dims(n).build()?;
  let a = Buffer::<f32>::builder().queue(pq.queue().clone()).len(m*k).fill_val(1.0f32).build()?;
  let b = Buffer::<f32>::builder().queue(pq.queue().clone()).len(k*n).fill_val(2.0f32).build()?;
  let c = Buffer::<f32>::builder().queue(pq.queue().clone()).len(m*n).fill_val(0.0f32).build()?;

  let ev = sgemm(
    pq.queue(),
    CLBlastLayout::RowMajor,
    CLBlastTranspose::No,
    CLBlastTranspose::No,
    m, n, k,
    1.0,
    &a, 0, k,
    &b, 0, n,
    0.0,
    &c, 0, n,
    &[])?;
  Ok(())
}
```

## options
```bash
# force re-build binding_static.rs and clblast_ocl_wrap.rs
cargo build --features generate-bindings
```
