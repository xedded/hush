//! Dev probe: loads the speaker model in tract and times one embedding.
use tract_onnx::prelude::*;

fn main() -> TractResult<()> {
    let t0 = std::time::Instant::now();
    let model = tract_onnx::onnx()
        .model_for_path(std::env::args().nth(1).as_deref().unwrap_or("models/campplus_lm.onnx"))?
        .with_input_fact(0, f32::fact([1, 150, 80]).into())?
        .into_optimized()?
        .into_runnable()?;
    println!("load+optimize: {:?}", t0.elapsed());
    let feats: Tensor = tract_ndarray::Array3::<f32>::from_shape_fn((1, 150, 80), |(_, t, f)| ((t * 7 + f * 3) % 11) as f32 * 0.1).into();
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let out = model.run(tvec!(feats.clone().into()))?;
        println!("run: {:?} shape {:?}", t.elapsed(), out[0].shape());
    }
    Ok(())
}
