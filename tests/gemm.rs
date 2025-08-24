// ocl と、作成したクレートのラッパー関数を use
use clblast_binding::{self};
use ocl::ProQue;

#[cfg(test)]
mod tests {
  use clblast_binding::{
    self,
    clblast_sys::{
      self,
      CLBlastLayout_::{CLBlastLayoutColMajor, CLBlastLayoutRowMajor},
      CLBlastTranspose_::CLBlastTransposeNo,
    },
  };
  // ProQueの代わりに、必要なoclコンポーネントを直接インポート
  use ocl::{Buffer, Context, Device, Platform, Queue};

  fn gemm_cpu_ref(m: usize, n: usize, k: usize, a: &[f32], b: &[f32]) -> Vec<f32> {
    let mut c = vec![0f32; m * n];
    for i in 0..m {
      for j in 0..n {
        let mut acc = 0f32;
        for p in 0..k {
          acc += a[i * k + p] * b[p * n + j];
        }
        c[i * n + j] = acc;
      }
    }
    c
  }

  #[test]
  fn sgemm_small_with_clblast_and_ocl() -> ocl::Result<()> {
    // 1) Platform, Device, Context, Queue を手動で初期化
    let platform = Platform::default();
    let device = Device::first(platform)?;
    let context = Context::builder()
      .platform(platform)
      .devices(device)
      .build()?;
    let queue = Queue::new(&context, device, None)?;

    // 2) データ準備 (変更なし)
    let (m, n, k) = (2usize, 3usize, 4usize);
    let a_host: Vec<f32> = (0..(m * k)).map(|i| i as f32).collect();
    let b_host: Vec<f32> = (0..(k * n)).map(|i| (i as f32) * 0.5).collect();
    let mut c_host: Vec<f32> = vec![0.0; m * n];

    let lda = k;
    let ldb = n;
    let ldc = n;

    // 3) ocl::Buffer の作成 (ProQueではなくQueueを直接利用)
    let a_buffer = Buffer::builder()
      .queue(queue.clone())
      .len(m * k)
      .copy_host_slice(&a_host)
      .build()?;
    let b_buffer = Buffer::builder()
      .queue(queue.clone())
      .len(k * n)
      .copy_host_slice(&b_host)
      .build()?;
    let mut c_buffer = Buffer::builder().queue(queue.clone()).len(m * n).build()?;

    // 4) 作成した安全なラッパー関数を呼び出す (queueを渡すように変更)
    let _ = clblast_binding::sgemm(
      &queue,
      CLBlastLayoutRowMajor,
      CLBlastTransposeNo,
      CLBlastTransposeNo,
      m,
      n,
      k,
      1.0,
      &a_buffer,
      0usize,
      k,
      &b_buffer,
      0usize,
      n,
      0.0,
      &c_buffer,
      0usize,
      n,
      &[],
    );

    // 5) 結果を読み出して検証 (変更なし)
    c_buffer.read(&mut c_host).enq()?;

    let c_ref = gemm_cpu_ref(m, n, k, &a_host, &b_host);
    for (i, (&x, &y)) in c_host.iter().zip(c_ref.iter()).enumerate() {
      assert!((x - y).abs() < 1e-4, "mismatch at {i}: got {x}, expect {y}");
    }

    Ok(())
  }
}
