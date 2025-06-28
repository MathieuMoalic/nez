use nalgebra::Vector3;
use rayon::prelude::*;
use std::{fs, sync::Arc};

// ---- Zarr stuff -----------------------------------------------------------
use zarrs::{
    array::{
        ArrayBuilder, DataType, FillValue, codec::array_to_bytes::sharding::ShardingCodecBuilder,
        codec::bytes_to_bytes::gzip::GzipCodec,
    },
    array_subset::ArraySubset,
    filesystem::FilesystemStore,
    group::GroupBuilder,
    storage::ReadableWritableListableStorage, // <- correct path
};
// ---------------------------------------------------------------------------

/// ---------------- Simulation parameters ----------------
const N_SPINS: usize = 128; // chain length
const D: f64 = 2.5e-9; // spacing (m)
const GAMMA: f64 = 1.760_859e11; // rad s⁻¹ T⁻¹
const ALPHA: f64 = 0.2; // damping
const A_EX: f64 = 1.3e-11; // exchange stiffness (J m⁻¹)
const MU0_MS: f64 = 4.0 * std::f64::consts::PI * 1.0e5; // μ₀Mₛ (≈ 1 T)

const DT: f64 = 1e-14; // time-step (s)
const N_STEPS: u64 = 50; // #time-steps

/// external field (constant here)
const H_EXT: Vector3<f64> = Vector3::new(0.0, 0.0, 1.0); // Tesla

/// LLG right-hand side for a single spin
#[inline(always)]
fn llg_rhs(m: &Vector3<f64>, h_eff: &Vector3<f64>) -> Vector3<f64> {
    let mxh = m.cross(h_eff);
    let mxmxh = m.cross(&mxh);
    let pref = -GAMMA / (1.0 + ALPHA * ALPHA);
    pref * (mxh + ALPHA * mxmxh)
}

/// Exchange field at site *i* (free boundaries)
fn exchange_field(chain: &[Vector3<f64>], i: usize) -> Vector3<f64> {
    let m_i = chain[i];
    let m_ip1 = if i + 1 < chain.len() {
        chain[i + 1]
    } else {
        chain[i]
    };
    let m_im1 = if i > 0 { chain[i - 1] } else { chain[i] };
    let lap = m_ip1 - 2.0 * m_i + m_im1;
    (2.0 * A_EX / MU0_MS) * lap / (D * D)
}

/// One RK4 step for the whole chain
fn rk4_step(chain: &[Vector3<f64>]) -> Vec<Vector3<f64>> {
    // k1
    let k1: Vec<_> = chain
        .par_iter()
        .enumerate()
        .map(|(i, m)| llg_rhs(m, &(H_EXT + exchange_field(chain, i))))
        .collect();

    // k2
    let tmp: Vec<_> = chain
        .iter()
        .zip(&k1)
        .map(|(m, k)| m + 0.5 * DT * (*k))
        .collect();
    let k2: Vec<_> = tmp
        .par_iter()
        .enumerate()
        .map(|(i, m)| llg_rhs(m, &(H_EXT + exchange_field(&tmp, i))))
        .collect();

    // k3
    let tmp: Vec<_> = chain
        .iter()
        .zip(&k2)
        .map(|(m, k)| m + 0.5 * DT * (*k))
        .collect();
    let k3: Vec<_> = tmp
        .par_iter()
        .enumerate()
        .map(|(i, m)| llg_rhs(m, &(H_EXT + exchange_field(&tmp, i))))
        .collect();

    // k4
    let tmp: Vec<_> = chain.iter().zip(&k3).map(|(m, k)| m + DT * (*k)).collect();
    let k4: Vec<_> = tmp
        .par_iter()
        .enumerate()
        .map(|(i, m)| llg_rhs(m, &(H_EXT + exchange_field(&tmp, i))))
        .collect();

    // final update + renormalise
    chain
        .iter()
        .zip(&k1)
        .zip(&k2)
        .zip(&k3)
        .zip(&k4)
        .map(|((((m, k1), k2), k3), k4)| {
            let next = *m + (DT / 6.0) * (*k1 + 2.0 * (*k2) + 2.0 * (*k3) + *k4);
            next.normalize()
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---------- initial state: small tilt ----------
    let tilt = 10f64.to_radians();
    let mut chain = vec![Vector3::new(tilt.sin(), 0.0, tilt.cos()); N_SPINS];

    // ---------- create Zarr store + dataset ----------
    let store_path = "magnetization.zarr";

    // If the folder already exists, delete it first
    if std::path::Path::new(store_path).exists() {
        fs::remove_dir_all(store_path)?;
    }

    let store: ReadableWritableListableStorage = Arc::new(FilesystemStore::new(store_path)?);

    // root group
    GroupBuilder::new()
        .build(store.clone(), "/")?
        .store_metadata()?;

    // shape: (time, z, y, x, vec)  →  (N_STEPS+1, N_SPINS, 1, 1, 3)
    let shape = vec![(N_STEPS + 1) as u64, 1, 1, N_SPINS as u64, 3];
    let chunk = vec![1, 1, 1, N_SPINS as u64, 3].try_into().unwrap();

    let mut sharding_codec_builder = ShardingCodecBuilder::new(
        vec![1, 1, 1, N_SPINS as u64, 3].try_into()?, // inner chunk shape
    );
    sharding_codec_builder.bytes_to_bytes_codecs(vec![Arc::new(GzipCodec::new(5)?)]);

    let array = ArrayBuilder::new(shape, DataType::Float64, chunk, FillValue::from(0.0f64))
        .array_to_bytes_codec(sharding_codec_builder.build_arc())
        .build(store.clone(), "/m")?;

    array.store_metadata()?; // write metadata once

    // ---------- time loop ----------
    for step in 0..=N_STEPS {
        let t = step as f64 * DT;

        // ---- write one time slice to Zarr ----
        let mut flat = Vec::<f64>::with_capacity(N_SPINS * 3);
        for m in &chain {
            flat.extend_from_slice(&[m.x, m.y, m.z]); // x, y, z
        }

        let subset = ArraySubset::new_with_ranges(&[
            step as u64..step as u64 + 1, // time
            0..N_SPINS as u64,            // z
            0..1,                         // y
            0..1,                         // x
            0..3,                         // vec
        ]);

        array.store_array_subset_elements(&subset, &flat)?;

        if step % 50 == 0 {
            let m_avg_z = chain.iter().map(|m| m.z).sum::<f64>() / N_SPINS as f64;
            println!("{:.3e}\t{:.6e}", t, m_avg_z);
        }

        chain = rk4_step(&chain);
    }

    Ok(())
}
