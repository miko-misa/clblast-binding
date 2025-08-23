#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
#![allow(clippy::all)]

// 既定は同梱の静的バインディングを利用
#[cfg(not(feature = "generate-bindings"))]
include!("bindings_static.rs");

#[cfg(feature = "generate-bindings")]
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
