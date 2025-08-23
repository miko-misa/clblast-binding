#[cfg(test)]
mod tests {
  use std::ffi::c_void;
  use std::ptr::{null, null_mut};

  // ★ クレートの FFI バインディングを「実際に」使うことが重要
  //    （これで build.rs の rustc-link-lib 指示が最終リンクに伝播します）
  use clblast_binding::{_cl_event, CLBlastSgemm};

  // --- （任意の保険）macOS でフレームワークを明示リンクしたい場合 ---
  #[cfg(target_os = "macos")]
  #[link(name = "OpenCL", kind = "framework")]
  unsafe extern "C" {}

  // ---- 最小限の OpenCL FFI（unsafe extern 必須：Rust 2024）----
  type cl_int = i32;
  type cl_uint = u32;
  type cl_bool = cl_uint;
  type cl_bitfield = u64;
  type cl_device_type = cl_bitfield;
  type cl_platform_id = *mut c_void;
  type cl_device_id = *mut c_void;
  type cl_context = *mut c_void;
  type cl_command_queue = *mut c_void;
  type cl_mem = *mut c_void;
  type cl_event = *mut c_void;
  type cl_context_properties = isize;

  const CL_SUCCESS: cl_int = 0;
  const CL_TRUE: cl_bool = 1;

  const CL_DEVICE_TYPE_GPU: cl_device_type = 1 << 2;
  const CL_DEVICE_TYPE_CPU: cl_device_type = 1 << 1;

  const CL_MEM_READ_ONLY: cl_bitfield = 1 << 2;
  const CL_MEM_READ_WRITE: cl_bitfield = 1 << 0;
  const CL_MEM_COPY_HOST_PTR: cl_bitfield = 1 << 5;

  // 2024 Edition: extern ブロックは unsafe 必須
  unsafe extern "C" {
    fn clGetPlatformIDs(
      num_entries: cl_uint,
      platforms: *mut cl_platform_id,
      num_platforms: *mut cl_uint,
    ) -> cl_int;

    fn clGetDeviceIDs(
      platform: cl_platform_id,
      device_type: cl_device_type,
      num_entries: cl_uint,
      devices: *mut cl_device_id,
      num_devices: *mut cl_uint,
    ) -> cl_int;

    fn clCreateContext(
      properties: *const cl_context_properties,
      num_devices: cl_uint,
      devices: *const cl_device_id,
      pfn_notify: Option<extern "C" fn(*const i8, *const c_void, usize, *mut c_void)>,
      user_data: *mut c_void,
      errcode_ret: *mut cl_int,
    ) -> cl_context;

    fn clCreateCommandQueue(
      context: cl_context,
      device: cl_device_id,
      properties: cl_bitfield,
      errcode_ret: *mut cl_int,
    ) -> cl_command_queue;

    fn clCreateBuffer(
      context: cl_context,
      flags: cl_bitfield,
      size: usize,
      host_ptr: *mut c_void,
      errcode_ret: *mut cl_int,
    ) -> cl_mem;

    fn clEnqueueWriteBuffer(
      queue: cl_command_queue,
      buffer: cl_mem,
      blocking: cl_bool,
      offset: usize,
      cb: usize,
      ptr: *const c_void,
      num_wait: cl_uint,
      wait_list: *const cl_event,
      evt: *mut cl_event,
    ) -> cl_int;

    fn clEnqueueReadBuffer(
      queue: cl_command_queue,
      buffer: cl_mem,
      blocking: cl_bool,
      offset: usize,
      cb: usize,
      ptr: *mut c_void,
      num_wait: cl_uint,
      wait_list: *const cl_event,
      evt: *mut cl_event,
    ) -> cl_int;

    fn clFinish(queue: cl_command_queue) -> cl_int;
    fn clReleaseMemObject(obj: cl_mem) -> cl_int;
    fn clReleaseCommandQueue(q: cl_command_queue) -> cl_int;
    fn clReleaseContext(ctx: cl_context) -> cl_int;
    fn clReleaseEvent(evt: cl_event) -> cl_int;
  }

  // CLBlast のレイアウト/転置（C API の数値を直値で）
  const CLBLAST_LAYOUT_ROW_MAJOR: u32 = 101;
  const CLBLAST_TRANSPOSE_NO: u32 = 111;

  fn gemm_cpu_ref(m: usize, n: usize, k: usize, a: &[f32], b: &[f32]) -> Vec<f32> {
    let mut c = vec![0f32; m * n];
    for i in 0..m {
      for j in 0..n {
        let mut acc = 0f32;
        for p in 0..k {
          acc += a[i * k + p] * b[p * n + j];
        }
        c[i * n + j] = acc; // beta=0
      }
    }
    c
  }

  #[test]
  fn sgemm_small_with_clblast() {
    unsafe {
      // 1) Platform/Device
      let mut nplat: cl_uint = 0;
      assert_eq!(clGetPlatformIDs(0, null_mut(), &mut nplat), CL_SUCCESS);
      assert!(nplat > 0);

      let mut plats = vec![null_mut(); nplat as usize];
      assert_eq!(
        clGetPlatformIDs(nplat, plats.as_mut_ptr(), null_mut()),
        CL_SUCCESS
      );

      let mut dev: cl_device_id = null_mut();
      let mut e = clGetDeviceIDs(plats[0], CL_DEVICE_TYPE_GPU, 1, &mut dev, null_mut());
      if e != CL_SUCCESS {
        e = clGetDeviceIDs(plats[0], CL_DEVICE_TYPE_CPU, 1, &mut dev, null_mut());
        assert_eq!(e, CL_SUCCESS, "No suitable device");
      }

      // 2) Context/Queue
      let mut err: cl_int = 0;
      let ctx = clCreateContext(null(), 1, &dev, None, null_mut(), &mut err);
      assert_eq!(err, CL_SUCCESS);
      let q = clCreateCommandQueue(ctx, dev, 0, &mut err);
      assert_eq!(err, CL_SUCCESS);

      // 3) Data
      let (m, n, k) = (2usize, 3usize, 4usize);
      let a: Vec<f32> = (0..(m * k)).map(|i| i as f32).collect();
      let b: Vec<f32> = (0..(k * n)).map(|i| (i as f32) * 0.5).collect();
      let mut c: Vec<f32> = vec![0.0; m * n];

      let lda = k;
      let ldb = n;
      let ldc = n;

      // 4) Buffers
      let mut o = 0;
      let buf_a = clCreateBuffer(
        ctx,
        CL_MEM_READ_ONLY | CL_MEM_COPY_HOST_PTR,
        a.len() * std::mem::size_of::<f32>(),
        a.as_ptr() as *mut c_void,
        &mut o,
      );
      assert_eq!(o, CL_SUCCESS);
      let buf_b = clCreateBuffer(
        ctx,
        CL_MEM_READ_ONLY | CL_MEM_COPY_HOST_PTR,
        b.len() * std::mem::size_of::<f32>(),
        b.as_ptr() as *mut c_void,
        &mut o,
      );
      assert_eq!(o, CL_SUCCESS);
      let buf_c = clCreateBuffer(
        ctx,
        CL_MEM_READ_WRITE,
        c.len() * std::mem::size_of::<f32>(),
        null_mut(),
        &mut o,
      );
      assert_eq!(o, CL_SUCCESS);

      // 5) CLBlast (クレートのバインディング経由！)
      let mut evt: *mut _cl_event = null_mut();
      let stat = CLBlastSgemm(
        CLBLAST_LAYOUT_ROW_MAJOR,
        CLBLAST_TRANSPOSE_NO,
        CLBLAST_TRANSPOSE_NO,
        m,
        n,
        k,
        1.0, // alpha
        buf_a as *mut _,
        0,
        lda,
        buf_b as *mut _,
        0,
        ldb,
        0.0, // beta
        buf_c as *mut _,
        0,
        ldc,
        &q as *const _ as *mut _,
        &mut evt,
      );
      assert_eq!(stat, 0, "CLBlastSgemm failed with status={}", stat);
      clFinish(q);
      if !evt.is_null() {
        clReleaseEvent(evt as *mut c_void);
      }

      // 6) Read back & check
      assert_eq!(
        clEnqueueReadBuffer(
          q,
          buf_c,
          CL_TRUE,
          0,
          c.len() * std::mem::size_of::<f32>(),
          c.as_mut_ptr() as *mut c_void,
          0,
          null(),
          null_mut()
        ),
        CL_SUCCESS
      );

      let cref = gemm_cpu_ref(m, n, k, &a, &b);
      for (i, (&x, &y)) in c.iter().zip(cref.iter()).enumerate() {
        assert!((x - y).abs() < 1e-4, "mismatch at {i}: got {x}, expect {y}");
      }

      // 7) Cleanup
      clReleaseMemObject(buf_a);
      clReleaseMemObject(buf_b);
      clReleaseMemObject(buf_c);
      clReleaseCommandQueue(q);
      clReleaseContext(ctx);
    }
  }
}
