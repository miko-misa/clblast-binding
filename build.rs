// build.rs
use std::{
  env, fs, io,
  path::{Path, PathBuf},
};

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
  // 既定: src/bindings_static.rs を使う。フラグON/未同梱時は bindgen で生成。
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
      .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

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
}
