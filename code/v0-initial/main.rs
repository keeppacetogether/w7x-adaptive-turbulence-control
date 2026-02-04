use ndarray::Array1;
use std::fs::File;
use std::io::{BufWriter, Write};
#[derive(Clone, Copy, PartialEq, Debug)]
enum ConfinementMode {
    Normal,          // 정상 운전 (난류 억제됨)
    TurbulencePulse, // 난류 펄스 (불순물 배출 모드)
}

struct StellaratorState {
    // 공간 그리드
    radius_grid: Array1<f64>,
    dr: f64,
    nr: usize,

    // 물리량
    impurity_density: Array1<f64>,
    electron_density: Array1<f64>,
    electron_temp: Array1<f64>,

    // 수송 계수
    d_neo: f64,
    d_turb_base: f64,
    v_neo: f64,

    // 제어 상태
    confinement_mode: ConfinementMode,
    time: f64,
    pulse_start_time: Option<f64>,

    // 진단 데이터 (히스토리)
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
            d_neo: 0.05,      // 신고전 확산 계수
            d_turb_base: 4.0, // 기본 난류 확산 계수
            v_neo: -0.8,      // 안쪽으로 향하는 대류 속도
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
            // 전자 밀도 및 온도 (Parabolic profile)
            self.electron_density[i] = 8e19 * (1.0 - r.powi(2));
            self.electron_temp[i] = 8.0 * (1.0 - r.powi(2));
            // 초기 불순물: 가장자리에 분포
            self.impurity_density[i] = 1e18 * (0.1 + 0.9 * r.powi(4));
        }
    }

    fn calculate_turbulence_level(&self, r_idx: usize) -> f64 {
        let r = self.radius_grid[r_idx];
        if r < 0.02 || r > 0.98 {
            return 0.05;
        }

        // ITG 특성화를 위한 기울기 계산 (중심 차분)
        let dn_dr =
            (self.electron_density[r_idx + 1] - self.electron_density[r_idx - 1]) / (2.0 * self.dr);
        let dt_dr =
            (self.electron_temp[r_idx + 1] - self.electron_temp[r_idx - 1]) / (2.0 * self.dr);

        // Gradient Scale Lengths (분모 0 방지)
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
