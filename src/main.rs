use nalgebra::Vector3;

/// Right-hand side of the LLG ODE (unit magnetisation `m`)
fn llg_rhs(m: &Vector3<f64>,
           h_eff: &Vector3<f64>,
           gamma: f64,
           alpha: f64) -> Vector3<f64>
{
    // m × H_eff
    let mxh     = m.cross(h_eff);
    // m × (m × H_eff)
    let mxmxh   = m.cross(&mxh);

    // prefactor γ/(1+α²)
    let pref = -gamma / (1.0 + alpha * alpha);

    pref * (mxh + alpha * mxmxh)
}

/// One fourth-order Runge–Kutta step with optional renormalisation
fn rk4_step(m: &Vector3<f64>,
            h_eff: &Vector3<f64>,
            dt: f64,
            gamma: f64,
            alpha: f64) -> Vector3<f64>
{
    let k1 = llg_rhs(m,                    h_eff, gamma, alpha);
    let k2 = llg_rhs(&(m + 0.5*dt*k1),     h_eff, gamma, alpha);
    let k3 = llg_rhs(&(m + 0.5*dt*k2),     h_eff, gamma, alpha);
    let k4 = llg_rhs(&(m + dt*k3),         h_eff, gamma, alpha);

    // classical RK-4 update
    let next = m + (dt/6.0)*(k1 + 2.0*k2 + 2.0*k3 + k4);

    // numerical drift can violate |m|=1 – renormalise explicitly
    next.normalize()
}

fn main() {
    // ---------- material & simulation parameters ----------
    let gamma: f64 = 2.211e5;    // (m A⁻¹ s⁻¹)
    let alpha: f64 = 0.1;        // dimensionless damping
    let h_eff = Vector3::new(0.0, 0.0, 1.0);   // constant field along +z (Tesla)

    let dt   = 1e-12;            // time step (s)
    let n_steps = 10_000;        // integrate to n_steps*dt

    // ---------- initial state ----------
    // start 30° away from the field direction
    let mut m = Vector3::new( (30f64.to_radians()).sin(), 0.0, (30f64.to_radians()).cos() );

    // ---------- time loop ----------
    for step in 0..=n_steps {
        let t = step as f64 * dt;
        println!("{:.3e}\t{:.6e}\t{:.6e}\t{:.6e}", t, m.x, m.y, m.z);

        m = rk4_step(&m, &h_eff, dt, gamma, alpha);
    }
}
