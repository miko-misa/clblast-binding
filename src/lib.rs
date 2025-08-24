#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
#![allow(clippy::all)]

pub mod clblast_sys {
  include!("bindings_static.rs");
}
include!("clblast_ocl_wrap.rs");
