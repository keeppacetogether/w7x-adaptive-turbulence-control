use ndarray::Array1;
use std::fs::File;
use std::io::{BufWriter, Write};
#[derive(Clone, Copy, PartialEq, Debug)]
enum ConfinementMode {
    Normal,          // Normal operation (turbulence suppressed)
    TurbulencePulse, // Turbulence pulse (impurity exhaust mode)
}

struct StellaratorState {
    // Spatial grid
    radius_grid: Array1<f64>,
    dr: f64,
    nr: usize,

    // Physical quantities
    impurity_density: Array1<f64>,
    electron_density: Array1<f64>,
    electron_temp: Array1<f64>,

    // Transport coefficients
    d_neo: f64,
    d_turb_base: f64,
    v_neo: f64,

    // Control state
    confinement_mode: ConfinementMode,
    time: f64,
    pulse_start_time: Option<f64>,

    // Diagnostic data (history)
    center_impurity_history: Vec<f64>,
    edge_impurity_history: Vec<f64>,
    turbulence_history: Vec<f64>,
    time_history: Vec<f64>,
}

impl StellaratorState {
    fn new(nr: usize) -> Self {
        let dr = 1.0 / (nr - 1) as f64;
        let radius_grid = Array1::linspace(0.0, 1.0, nr);

        let mut state = StellaratorState {
            radius_grid,
            dr,
            nr,
            impurity_density: Array1::zeros(nr),
            electron_density: Array1::zeros(nr),
            electron_temp: Array1::zeros(nr),
            d_neo: 0.05,      // Neoclassical diffusion coefficient
            d_turb_base: 4.0, // Base turbulent diffusion coefficient
            v_neo: -0.8,      // Inward convective velocity
            confinement_mode: ConfinementMode::Normal,
            time: 0.0,
            pulse_start_time: None,
            center_impurity_history: Vec::new(),
            edge_impurity_history: Vec::new(),
            turbulence_history: Vec::new(),
            time_history: Vec::new(),
        };

        state.initialize_profiles();
        state
    }

    fn initialize_profiles(&mut self) {
        for (i, &r) in self.radius_grid.iter().enumerate() {
            // Electron density and temperature (Parabolic profile)
            self.electron_density[i] = 8e19 * (1.0 - r.powi(2));
            self.electron_temp[i] = 8.0 * (1.0 - r.powi(2));
            // Initial impurity: distributed at the edge
            self.impurity_density[i] = 1e18 * (0.1 + 0.9 * r.powi(4));
        }
    }

    fn calculate_turbulence_level(&self, r_idx: usize) -> f64 {
        let r = self.radius_grid[r_idx];
        if r < 0.02 || r > 0.98 {
            return 0.05;
        }

        // Calculate gradient for ITG characterization (central difference)
        let dn_dr =
            (self.electron_density[r_idx + 1] - self.electron_density[r_idx - 1]) / (2.0 * self.dr);
        let dt_dr =
            (self.electron_temp[r_idx + 1] - self.electron_temp[r_idx - 1]) / (2.0 * self.dr);

        // Gradient Scale Lengths (prevent division by zero)
        let ln = -(self.electron_density[r_idx] / dn_dr.min(-1e-10)).abs();
        let lt = -(self.electron_temp[r_idx] / dt_dr.min(-1e-10)).abs();
        let eta = (ln / lt).max(0.1).min(10.0);

        let factor = match self.confinement_mode {
            ConfinementMode::Normal => {
                if eta > 0.8 && eta < 1.2 {
                    0.2
                } else {
                    1.0
                }
            }
            ConfinementMode::TurbulencePulse => {
                if r >
