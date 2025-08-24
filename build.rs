// build.rs
use std::{
  env, fs, io,
  path::{Path, PathBuf},
  process::Command,
};

use bindgen::EnumVariation;

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
  if !dst.exists() {
    fs::create_dir_all(dst)?;
  }
  for e in fs::read_dir(src)? {
    let e = e?;
    let p = e.path();
    let to = dst.join(e.file_name());
    if p.is_dir() {
      copy_dir(&p, &to)?;
    } else {
      fs::copy(&p, &to)?;
    }
  }
  Ok(())
}

fn format_rs_file(path: &Path) {
  // 1) rustfmt があれば使う（edition はプロジェクトに合わせて）
  if let Ok(status) = Command::new("rustfmt")
    .arg("--edition")
    .arg("2021")
    .arg("--color")
    .arg("never")
    .arg(path)
    .status()
  {
    if status.success() {
      return;
    }
  }
  // 2) フォールバック: prettyplease（コメントは消える可能性あり）
  if let Ok(src) = fs::read_to_string(path) {
    if let Ok(file) = syn::parse_file(&src) {
      let pretty = prettyplease::unparse(&file);
      let _ = fs::write(path, pretty);
    }
  }
}

fn main() {
  let target = env::var("TARGET").expect("TARGET not set");

  // ---- feature 判定（同時オンは禁止） ----
  let f_v_clb = env::var("CARGO_FEATURE_VENDORED_CLBLAST").is_ok();
  let f_s_clb = env::var("CARGO_FEATURE_SYSTEM_CLBLAST").is_ok();
  let f_v_ocl = env::var("CARGO_FEATURE_VENDORED_OPENCL_HEADERS").is_ok();
  let f_s_ocl = env::var("CARGO_FEATURE_SYSTEM_OPENCL_HEADERS").is_ok();
  let f_gen = env::var("CARGO_FEATURE_GENERATE_BINDINGS").is_ok();

  if f_v_clb && f_s_clb {
    panic!("features 'vendored-clblast' and 'system-clblast' are mutually exclusive");
  }
  if f_v_ocl && f_s_ocl {
    panic!("features 'vendored-opencl-headers' and 'system-opencl-headers' are mutually exclusive");
  }

  // ---- vendor ルート（環境変数で上書き可）----
  let vendor_root = PathBuf::from("vendor");
  let clblast_src = env::var("CLBLAST_SRC_DIR")
    .map(PathBuf::from)
    .unwrap_or(vendor_root.join("clblast"));
  let ocl_headers = env::var("OPENCL_HEADERS_DIR")
    .map(PathBuf::from)
    .unwrap_or(vendor_root.join("opencl_headers"));

  // ---- OpenCL headers: include ルートを用意（vendored の場合はシム生成）----
  let out = PathBuf::from(env::var("OUT_DIR").unwrap());
  let shim_root = out.join("sdkshims"); // <- bindgen / cmake 共通の include root
  let shim_opencl = shim_root.join("OpenCL");
  let shim_cl = shim_root.join("CL");
  fs::create_dir_all(&shim_opencl).unwrap();
  fs::create_dir_all(&shim_cl).unwrap();

  if f_v_ocl {
    // vendor の CL/* をコピーして、OpenCL/opencl.h は CL/cl.h を参照するシム
    let src_cl = ocl_headers.join("CL");
    if !src_cl.join("cl.h").exists() {
      panic!(
        "OpenCL headers not found at {:?}. Put Khronos OpenCL-Headers under vendor/opencl_headers.",
        src_cl
      );
    }
    copy_dir(&src_cl, &shim_cl).expect("copy CL headers failed");
    fs::write(shim_opencl.join("opencl.h"), "#include \"../CL/cl.h\"\n")
      .expect("write shim OpenCL/opencl.h failed");
  } else {
    // system ヘッダ: pkg-config / vcpkg から include パスを bindgen に渡す（後段）
  }

  // ---- CLBlast: system か vendored か ----
  let clblast_header: PathBuf;
  if f_s_clb {
    println!("cargo:info=Using system CLBlast (dynamic)");
    if target.contains("windows") {
      let lib = vcpkg::find_package("clblast")
        .expect("vcpkg: CLBlast not found. `vcpkg install clblast opencl`");
      clblast_header = lib
        .include_paths
        .get(0)
        .expect("vcpkg: include path missing")
        .join("clblast_c.h");
      // vcpkg は自動でリンク指示を出す
    } else {
      let lib = pkg_config::Config::new()
        .atleast_version("1.5")
        .probe("clblast")
        .expect("pkg-config: CLBlast not found (install libclblast-dev or provide .pc)");
      clblast_header = lib
        .include_paths
        .get(0)
        .expect("pkg-config: include path missing")
        .join("clblast_c.h");
      // pkg-config が -L/-l を出す
    }
    if !clblast_header.exists() {
      panic!("clblast_c.h not found at {:?}", clblast_header);
    }
  } else {
    println!("cargo:info=Building bundled CLBlast (static)");
    let mut cfg = cmake::Config::new(&clblast_src);
    cfg.define("BUILD_SHARED_LIBS", "OFF"); // 静的ライブラリ

    // CMake/FindOpenCL にヒント：include は shim_root を渡す
    cfg.define("OpenCL_INCLUDE_DIR", &shim_root);

    // OpenCL_LIBRARY のヒント（macOS は Framework）
    if target.contains("apple") {
      cfg.define(
        "OpenCL_LIBRARY",
        "/System/Library/Frameworks/OpenCL.framework/OpenCL",
      );
    } else if let Ok(libdir) = env::var("OPENCL_LIB_DIR") {
      cfg.define("OpenCL_LIBRARY", Path::new(&libdir));
    }
    let dst = cfg.build();
    let libdir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", libdir.display());
    println!("cargo:rustc-link-lib=static=clblast");

    clblast_header = clblast_src.join("include/clblast_c.h");
    if !clblast_header.exists() {
      panic!("clblast_c.h not found at {:?}", clblast_header);
    }
  }

  // ---- ランタイムのリンク（OpenCL / C++）----
  if target.contains("apple") {
    // macOS: OpenCL Framework + libc++
    println!("cargo:rustc-link-lib=framework=OpenCL");
    println!("cargo:rustc-link-lib=dylib=c++");
  } else if target.contains("windows") {
    // Windows: OpenCL (MSVC=OpenCL.lib / MinGW=libOpenCL.dll.a)
    println!("cargo:rustc-link-lib=dylib=OpenCL");
    if target.contains("gnu") {
      println!("cargo:rustc-link-lib=dylib=stdc++");
    }
  } else {
    // Linux / other Unix
    println!("cargo:rustc-link-lib=dylib=OpenCL");
    println!("cargo:rustc-link-lib=dylib=stdc++");
  }

  // ---- バインディング（静的 or 生成）----
  let static_rs = PathBuf::from("src").join("bindings_static.rs");
  let need_generate = f_gen || !static_rs.exists();

  let out_bind = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");

  if need_generate {
    // --- bindgen 実行 ---
    println!("cargo:info=Generating bindings with bindgen (generate-bindings or no static file)");
    println!("cargo:rerun-if-changed={}", clblast_header.display());

    let mut b = bindgen::Builder::default()
      .header(clblast_header.to_string_lossy())
      .allowlist_function("CLBlast.*")
      .allowlist_type("CLBlast.*")
      .allowlist_var("CLBlast.*")
      .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
      .size_t_is_usize(true)
      .rustfmt_bindings(true)
      .size_t_is_usize(true)
      .formatter(bindgen::Formatter::Rustfmt)
      .prepend_enum_name(false)
      .rustified_enum("CLBlast.*");

    // 常に shim_root を優先（OpenCL/opencl.h を見せる）
    b = b.clang_arg(format!("-I{}", shim_root.display()));

    if f_v_ocl {
      // vendor ヘッダ（CL/*）も見せる
      b = b.clang_arg(format!("-I{}", ocl_headers.display()));
      b = b.clang_arg(format!("-I{}", ocl_headers.join("CL").display()));
    } else {
      // system ヘッダ: Unix は pkg-config, Windows は vcpkg から include を得る
      if target.contains("windows") {
        if let Ok(ocl) = vcpkg::find_package("opencl") {
          for p in ocl.include_paths {
            b = b.clang_arg(format!("-I{}", p.display()));
          }
        }
      } else if let Ok(ocl) = pkg_config::Config::new().probe("OpenCL") {
        for p in ocl.include_paths {
          b = b.clang_arg(format!("-I{}", p.display()));
        }
      }
    }

    // macOS: 念のため Framework 検索も追加（libclang 向け）
    if target.contains("apple") {
      b = b.clang_arg("-F/System/Library/Frameworks");
      b = b.clang_arg("-I/System/Library/Frameworks/OpenCL.framework/Headers");
    }

    let bindings = b.generate().expect("Unable to generate CLBlast bindings");
    bindings
      .write_to_file(&out_bind)
      .expect("Couldn't write bindings.rs");

    // 生成物を静的ファイルにも書き戻し（失敗してもビルドは継続）
    if let Err(e) =
      fs::create_dir_all("src").and_then(|_| fs::copy(&out_bind, &static_rs).map(|_| ()))
    {
      eprintln!("cargo:warning=failed to write src/bindings_static.rs: {e}");
    }
  } else {
    // --- 生成しない: 静的ファイルを OUT_DIR にコピーして lib.rs の include に備える ---
    println!("cargo:info=Using prebuilt static bindings (src/bindings_static.rs)");
    println!("cargo:rerun-if-changed={}", static_rs.display());
    fs::copy(&static_rs, &out_bind)
      .expect("Couldn't copy src/bindings_static.rs to OUT_DIR/bindings.rs");
  }

  // ---- ここから “全関数対応” ラッパ自動生成 ----

  let out_wrap_outdir = out.join("clblast_ocl_wrap.rs");
  generate_ocl_wrappers(&out_bind, &out_wrap_outdir);
  println!(
    "cargo:warning=CLBlast wrappers: generated -> {}",
    out_wrap_outdir.display()
  );

  format_rs_file(&out_wrap_outdir);

  // src へは「初回のみ」コピー。既にある場合は何もしない。
  // 更新したいときだけ CLBLAST_REFRESH_WRAPPERS=1 を付けてビルド。
  let wrap_static = PathBuf::from("src").join("clblast_ocl_wrap.rs");
  println!("cargo:rerun-if-env-changed=CLBLAST_REFRESH_WRAPPERS");
  if !wrap_static.exists() {
    if let Err(e) =
      fs::create_dir_all("src").and_then(|_| fs::copy(&out_wrap_outdir, &wrap_static).map(|_| ()))
    {
      eprintln!("cargo:warning=failed to write src/clblast_ocl_wrap.rs: {e}");
    } else {
      println!("cargo:info=Bootstrapped {}", wrap_static.display());
    }
  } else if env::var("CLBLAST_REFRESH_WRAPPERS").ok().as_deref() == Some("1") {
    // 差分があるときだけ上書き（無駄な更新を避ける）
    match (fs::read(&wrap_static), fs::read(&out_wrap_outdir)) {
      (Ok(old), Ok(new)) if old != new => {
        if let Err(e) = fs::write(&wrap_static, new) {
          eprintln!("cargo:warning=failed to update src/clblast_ocl_wrap.rs: {e}");
        } else {
          println!("cargo:info=Updated {}", wrap_static.display());
        }
      }
      _ => println!(
        "cargo:info=No wrapper changes; kept {}",
        wrap_static.display()
      ),
    }
  } else {
    println!(
      "cargo:info=Keeping existing {} (set CLBLAST_REFRESH_WRAPPERS=1 to refresh)",
      wrap_static.display()
    );
  }
}

// === ラッパ自動生成: OUT_DIR/bindings.rs -> OUT_DIR/clblast_ocl_wrap.rs ===
fn generate_ocl_wrappers(bindings_rs: &std::path::Path, out_wrappers: &std::path::Path) {
  use heck::ToSnakeCase;
  use quote::{format_ident, quote};
  use syn::{self, *};

  let src = std::fs::read_to_string(bindings_rs).expect("read bindings.rs failed");
  let file: syn::File = syn::parse_file(&src).expect("parse bindgen output failed");

  // 再エクスポートする定数と、生成するラッパ関数のバッファ
  let mut const_exports: Vec<proc_macro2::TokenStream> = Vec::new();
  let mut fn_wrappers: Vec<proc_macro2::TokenStream> = Vec::new();
  let mut wrapped_count = 0usize;

  // 型ヘルパ
  fn is_ident(ty: &Type, want: &str) -> bool {
    if let Type::Path(tp) = ty {
      if let Some(seg) = tp.path.segments.last() {
        return seg.ident == want;
      }
    }
    false
  }
  fn is_ptr_to(ty: &Type, want: &str) -> bool {
    if let Type::Ptr(p) = ty {
      return is_ident(&p.elem, want);
    }
    false
  }

  for item in file.items.iter() {
    // CLBlast 定数の再エクスポート
    if let Item::Const(ic) = item {
      if ic.ident.to_string().starts_with("CLBlast") {
        let ident = &ic.ident;
        const_exports.push(quote! { pub use crate::clblast_sys::#ident; });
      }
    }

    // extern "C" ブロック
    if let Item::ForeignMod(fm) = item {
      // bindgen は ABI 名を省略することがある（None = C 扱い）
      let abi_is_c = fm
        .abi
        .name
        .as_ref()
        .map(|n| n.value() == "C")
        .unwrap_or(true);
      if !abi_is_c {
        continue;
      }

      for it in fm.items.iter() {
        if let ForeignItem::Fn(f) = it {
          let cname = f.sig.ident.to_string();
          if !cname.starts_with("CLBlast") {
            continue;
          }

          // ラッパ関数名は CLBlastXxx -> xxx (snake_case)
          let wident = format_ident!("{}", cname.trim_start_matches("CLBlast").to_snake_case());
          let corename = format_ident!("{}", cname); // sys::CLBlastXxx

          // 引数を収集
          let mut args: Vec<(Ident, Type)> = Vec::new();
          for a in f.sig.inputs.iter() {
            if let FnArg::Typed(PatType { pat, ty, .. }) = a {
              if let Pat::Ident(pi) = &**pat {
                args.push((pi.ident.clone(), *(*ty).clone()));
              }
            }
          }

          // 末尾2引数が (*mut cl_command_queue, *mut cl_event) か？
          let (has_qe, qi, ei) = if args.len() >= 2 {
            let last = args.len() - 1;
            let prev = args.len() - 2;
            (
              is_ptr_to(&args[prev].1, "cl_command_queue") && is_ptr_to(&args[last].1, "cl_event"),
              prev,
              last,
            )
          } else {
            (false, 0, 0)
          };

          // 戻りが CLBlastStatusCode か？
          let returns_status =
            matches!(&f.sig.output, ReturnType::Type(_, ty) if is_ident(&*ty, "CLBlastStatusCode"));

          // ラッパの引数と実引数
          let mut wrapper_params: Vec<proc_macro2::TokenStream> = Vec::new();
          let mut call_args: Vec<proc_macro2::TokenStream> = Vec::new();
          let mut generics: Vec<proc_macro2::TokenStream> = Vec::new();
          let mut where_bounds: Vec<proc_macro2::TokenStream> = Vec::new();
          let mut t_idx = 0usize;

          for (i, (name, ty)) in args.iter().enumerate() {
            if has_qe && (i == qi || i == ei) {
              continue;
            }

            if is_ident(ty, "cl_mem") {
              t_idx += 1;
              let g = format_ident!("T{}", t_idx);
              wrapper_params.push(quote! { #name: &ocl::Buffer<#g> });
              call_args.push(quote! { to_mem(#name) });
              generics.push(quote! { #g });
              where_bounds.push(quote! { #g: ocl::OclPrm });
            } else {
              wrapper_params.push(quote! { #name: #ty });
              call_args.push(quote! { #name });
            }
          }

          if has_qe {
            wrapper_params.insert(0, quote! { queue: &ocl::Queue });
            wrapper_params.push(quote! { wait_for: &[CoreEvent] });
          }

          // 戻り型
          let wrapper_ret = if returns_status {
            if has_qe {
              quote! { ocl::Result<Option<CoreEvent>> }
            } else {
              quote! { ocl::Result<()> }
            }
          } else {
            match &f.sig.output {
              ReturnType::Default => quote! { () },
              ReturnType::Type(_, ty) => quote! { #ty },
            }
          };

          // 本体
          let body = if returns_status {
            if has_qe {
              quote! {
                let _marker = enqueue_marker_wait(queue, wait_for)?;
                let mut raw_ev: sys::cl_event = std::ptr::null_mut();
                let status = with_queue_ptr(queue, |qptr| unsafe {
                  sys::#corename(#(#call_args,)* qptr, &mut raw_ev as *mut _)
                });
                if !clblast_ok(status) {
                  return Err(ocl::Error::from(format!(concat!(stringify!(#corename), " failed: code={:?}"), status)));
                }
                Ok(unsafe { wrap_new_event(raw_ev) })
              }
            } else {
              quote! {
                let status = unsafe { sys::#corename(#(#call_args,)*) };
                if !clblast_ok(status) {
                  return Err(ocl::Error::from(format!(concat!(stringify!(#corename), " failed: code={:?}"), status)));
                }
                Ok(())
              }
            }
          } else {
            if has_qe {
              // 通常ここには来ないが形式的にサポート
              quote! {
                let _marker = enqueue_marker_wait(queue, wait_for)?;
                unsafe { sys::#corename(#(#call_args,)* std::ptr::null_mut(), std::ptr::null_mut()) }
              }
            } else {
              quote! { unsafe { sys::#corename(#(#call_args,)*) } }
            }
          };

          let gdef = if generics.is_empty() {
            quote! {}
          } else {
            quote! { <#(#generics,)*> }
          };
          let gwhr = if where_bounds.is_empty() {
            quote! {}
          } else {
            quote! { where #(#where_bounds,)* }
          };

          fn_wrappers.push(quote! {
            #[allow(clippy::too_many_arguments)]
            pub fn #wident #gdef ( #(#wrapper_params,)* ) -> #wrapper_ret #gwhr { #body }
          });
          wrapped_count += 1;
        }
      }
    }
  }

  // まとめて出力
  let out = quote! {
    // ===== AUTO-GENERATED: CLBlast ocl wrappers =====
    // このファイルは build.rs により自動生成されています。

    use crate::clblast_sys as sys;
    use ocl::core as ocore;
    use ocl::{Buffer, Queue};
    pub use ocore::Event as CoreEvent;
    use sys::*;
    #[inline]
    pub fn with_queue_ptr<R>(queue: &Queue, f: impl FnOnce(*mut cl_command_queue) -> R) -> R {
      let raw_cq_sys = queue.as_core().as_ptr();

      let mut cq_bindgen: cl_command_queue = raw_cq_sys as *mut _;
      let cq_ptr: *mut cl_command_queue = &mut cq_bindgen as *mut _;
      f(cq_ptr)
    }
    #[inline]
    fn to_mem<T: ocl::OclPrm>(buf: &Buffer<T>) -> sys::cl_mem {
      buf.as_core().as_ptr() as sys::cl_mem
    }
    #[inline]
    pub fn enqueue_marker_wait<'a>(
      queue: &ocl::Queue,
      wait_for: &[CoreEvent],
    ) -> ocl::Result<Option<CoreEvent>> {
      if wait_for.is_empty() {
        return Ok(None);
      }
      unsafe {
        let cq = queue.as_core().as_ptr();
        // Create a raw wait-list:
        let mut raw_events: Vec<cl_sys::cl_event> = Vec::with_capacity(wait_for.len());
        for e in wait_for {
          // Safety: just borrowing the inner pointer (no retain here).
          let ptr_ref = e.as_ptr_ref();
          raw_events.push(*ptr_ref);
        }
        let mut marker: cl_sys::cl_event = std::ptr::null_mut();
        let err = cl_sys::clEnqueueMarkerWithWaitList(
          cq,
          raw_events.len() as u32,
          raw_events.as_ptr(),
          &mut marker as *mut _,
        );
        if err != cl_sys::CL_SUCCESS as i32 {
          return Err(ocl::Error::from(format!(
            "clEnqueueMarkerWithWaitList failed: {}",
            err
          )));
        }
        // Wrap marker event:
        let ev = ocore::types::abs::Event::from_raw_create_ptr(marker);
        Ok(Some(ev))
      }
    }
    #[inline]
    fn clblast_ok(code: sys::CLBlastStatusCode) -> bool {
      (code as i32) == 0
    }
    #[inline]
    unsafe fn wrap_new_event(raw: sys::cl_event) -> Option<CoreEvent> {
      if raw.is_null() {
        None
      } else {
        let raw_sys = raw as cl_sys::cl_event;
        Some(ocore::types::abs::Event::from_raw_create_ptr(raw_sys))
      }
    }

    pub mod consts { #(#const_exports)* }

    #(#fn_wrappers)*
  };

  std::fs::write(out_wrappers, out.to_string()).expect("write clblast_ocl_wrap.rs failed");
  println!(
    "cargo:warning=CLBlast wrappers generated: {}",
    wrapped_count
  );
}
