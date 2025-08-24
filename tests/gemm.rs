// ocl と、作成したクレートのラッパー関数を use
use clblast_binding::{self};
use ocl::ProQue;

#[cfg(test)]
mod tests {
  use clblast_binding::{
    self,
    clblast_sys::{self, CLBlastLayout, CLBlastTranspose},
    sgemm,
  };
  // ProQueの代わりに、必要なoclコンポーネントを直接インポート
  use ocl::{Buffer, Context, Device, Platform, ProQue, Queue};

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
    let platform = Platform::default();
    let device = Device::first(platform)?;
    let context = Context::builder()
      .platform(platform)
      .devices(device)
      .build()?;
    let queue = Queue::new(&context, device, None)?;

    let (m, n, k) = (2usize, 3usize, 4usize);
    let a_host: Vec<f32> = (0..(m * k)).map(|i| i as f32).collect();
    let b_host: Vec<f32> = (0..(k * n)).map(|i| (i as f32) * 0.5).collect();
    let mut c_host: Vec<f32> = vec![0.0; m * n];

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
    let _ = sgemm(
      &queue,
      CLBlastLayout::RowMajor,
      CLBlastTranspose::No,
      CLBlastTranspose::No,
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

    c_buffer.read(&mut c_host).enq()?;

    let c_ref = gemm_cpu_ref(m, n, k, &a_host, &b_host);
    for (i, (&x, &y)) in c_host.iter().zip(c_ref.iter()).enumerate() {
      assert!((x - y).abs() < 1e-4, "mismatch at {i}: got {x}, expect {y}");
    }

    Ok(())
  }

  #[test]
  fn quick_start() -> ocl::Result<()> {
    let m = 2;
    let n = 2;
    let k = 2;
    let pq = ProQue::builder()
      .src("__kernel void nop(){}")
      .dims(n)
      .build()?;
    let a = Buffer::<f32>::builder()
      .queue(pq.queue().clone())
      .len(m * k)
      .fill_val(1.0f32)
      .build()?;
    let b = Buffer::<f32>::builder()
      .queue(pq.queue().clone())
      .len(k * n)
      .fill_val(2.0f32)
      .build()?;
    let c = Buffer::<f32>::builder()
      .queue(pq.queue().clone())
      .len(m * n)
      .fill_val(0.0f32)
      .build()?;

    let _ = sgemm(
      pq.queue(),
      CLBlastLayout::RowMajor,
      CLBlastTranspose::No,
      CLBlastTranspose::No,
      m,
      n,
      k,
      1.0,
      &a,
      0,
      k,
      &b,
      0,
      n,
      0.0,
      &c,
      0,
      n,
      &[],
    )?;
    Ok(())
  }
}
