use ndarray::Array1;
use std::fs::File;
use std::io::{BufWriter, Write};

#[derive(Clone, Copy, PartialEq, Debug)]
enum ConfinementMode {
    Normal,
    TurbulencePulse,
}

struct StellaratorState {
    radius_grid: Array1<f64>,
    dr: f64,
    nr: usize,
    impurity_density: Array1<f64>,
    electron_density: Array1<f64>,
    electron_temp: Array1<f64>,
    d_neo: f64,
    d_turb_base: f64,
    v_neo: f64,
    confinement_mode: ConfinementMode,
    time: f64,
    pulse_start_time: Option<f64>,
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
            d_neo: 0.02,      // ⭐ 줄임
            d_turb_base: 1.0, // ⭐ 줄임
            v_neo: -0.3,      // ⭐ 줄임
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
            self.electron_density[i] = 8e19 * (1.0 - r.powi(2));
            self.electron_temp[i] = 8.0 * (1.0 - r.powi(2));
            // 더 부드러운 초기 프로파일 ⭐
            self.impurity_density[i] = 1e18 * (0.2 + 0.8 * r.powi(2));
        }
    }

    fn calculate_turbulence_level(&self, r_idx: usize) -> f64 {
        let r = self.radius_grid[r_idx];
        if r < 0.02 || r > 0.98 {
            return 0.05;
        }

        let dn_dr = (self.electron_density[r_idx + 1] - self.electron_density[r_idx - 1]) 
                    / (2.0 * self.dr);
        let dt_dr = (self.electron_temp[r_idx + 1] - self.electron_temp[r_idx - 1]) 
                    / (2.0 * self.dr);

        // ⭐ 수정: abs + max
        let ln = (self.electron_density[r_idx] / dn_dr.abs().max(1e-10)).abs();
        let lt = (self.electron_temp[r_idx] / dt_dr.abs().max(1e-10)).abs();
        let eta = (ln / lt).max(0.1).min(10.0);

        let factor = match self.confinement_mode {
            ConfinementMode::Normal => {
                if eta > 0.8 && eta < 1.2 {
                    0.3  // ⭐ 0.2 → 0.3
                } else {
                    1.0
                }
            }
            ConfinementMode::TurbulencePulse => {
                if r > 0.7 { 
                    3.0  // ⭐ 5.0 → 3.0
                } else { 
                    1.0 
                }
            }
        };

        self.d_turb_base * factor
    }

    fn calculate_flux(&self, r_idx: usize) -> f64 {
        if r_idx == 0 || r_idx >= self.nr - 1 {
            return 0.0;
        }

        let n_z = self.impurity_density[r_idx];
        let dn_z_dr = (self.impurity_density[r_idx + 1] - self.impurity_density[r_idx - 1]) 
                      / (2.0 * self.dr);

        let d_total = self.d_neo + self.calculate_turbulence_level(r_idx);

        self.v_neo * n_z - d_total * dn_z_dr
    }

    fn detect_impurity_accumulation(&self) -> bool {
        let center_nz = self.impurity_density[0];
        
        if center_nz > 1.5e18 {  // ⭐ 2.2 → 1.5 (더 빨리 개입)
            return true;
        }

        if self.center_impurity_history.len() > 100 {
            let last = self.center_impurity_history.len() - 1;
            let prev = last - 100;
            let rate = (self.center_impurity_history[last] - self.center_impurity_history[prev])
                / (self.time_history[last] - self.time_history[prev]);
            if rate > 8e17 {  // ⭐ 1.5e18 → 8e17
                return true;
            }
        }
        false
    }

    fn update(&mut self, dt: f64) {
        // CFL 체크 추가 ⭐
        let d_max = match self.confinement_mode {
            ConfinementMode::Normal => self.d_turb_base,
            ConfinementMode::TurbulencePulse => self.d_turb_base * 3.0,
        };
        
        let cfl = d_max * dt / (self.dr * self.dr);
        if cfl > 0.5 && self.time < 0.001 {  // 초기에만 경고
            eprintln!("⚠️ CFL = {:.3} (dt={:.6}, dr={:.4}, D={:.2})", 
                     cfl, dt, self.dr, d_max);
        }

        // 제어 로직
        match self.confinement_mode {
            ConfinementMode::Normal => {
                if self.detect_impurity_accumulation() {
                    println!("⚠️ t={:.3}s: 불순물 축적! 펄스 시작", self.time);
                    self.confinement_mode = ConfinementMode::TurbulencePulse;
                    self.pulse_start_time = Some(self.time);
                }
            }
            ConfinementMode::TurbulencePulse => {
                if let Some(start) = self.pulse_start_time {
                    if self.time - start > 0.1 {  // ⭐ 0.15 → 0.1
                        self.confinement_mode = ConfinementMode::Normal;
                        self.pulse_start_time = None;
                        println!("✅ t={:.3}s: 정상 복귀", self.time);
                    }
                }
            }
        }

        // 수송 방정식
        let mut new_nz = self.impurity_density.clone();
        for i in 1..self.nr - 1 {
            let r = self.radius_grid[i];
            let flux_p = self.calculate_flux(i);
            let flux_m = self.calculate_flux(i - 1);

            let r_p = r + 0.5 * self.dr;
            let r_m = r - 0.5 * self.dr;

            // ⭐ 중심 처리 개선
            let div_flux = if r > 0.01 {
                (r_p * flux_p - r_m * flux_m) / (r * self.dr)
            } else {
                (flux_p - flux_m) / self.dr
            };
            
            let source = if r > 0.85 { 2e17 } else { 0.0 };  // ⭐ 3e17 → 2e17

            new_nz[i] = (self.impurity_density[i] + (-div_flux + source) * dt).max(0.0);
            
            // ⭐ 안전장치: 너무 큰 값 제한
            new_nz[i] = new_nz[i].min(1e20);
        }

        new_nz[0] = new_nz[1];
        new_nz[self.nr - 1] = 0.3 * new_nz[self.nr - 2];  // ⭐ 0.4 → 0.3

        self.impurity_density = new_nz;

        self.center_impurity_history.push(self.impurity_density[0]);
        self.edge_impurity_history.push(self.impurity_density[self.nr - 1]);
        self.turbulence_history.push(self.calculate_turbulence_level(self.nr - 2));
        self.time_history.push(self.time);

        self.time += dt;
    }

    fn save_to_csv(&self, filename: &str) -> std::io::Result<()> {
        let file = File::create(filename)?;
        let mut writer = BufWriter::new(file);

        writeln!(writer, "time,center_impurity,edge_impurity,turbulence")?;
        for i in 0..self.time_history.len() {
            writeln!(
                writer,
                "{:.6},{:.6e},{:.6e},{:.4}",
                self.time_history[i],
                self.center_impurity_history[i],
                self.edge_impurity_history[i],
                self.turbulence_history[i]
            )?;
        }
        Ok(())
    }
}

fn main() {
    println!("🌟 W7-X 적응형 난류 제어 시뮬레이터 v2.0 (안정화)");
    println!("{}", "=".repeat(60));

    let mut state = StellaratorState::new(101);

    let dt = 0.00002;  // ⭐ 50μs → 20μs
    let t_max = 10.0;
    let mut step = 0;

    println!("시뮬레이션 파라미터:");
    println!("  dt = {:.6}s, dr = {:.4}, nr = {}", dt, state.dr, state.nr);
    println!("  D_neo = {:.2}, D_turb_base = {:.2}, v_neo = {:.2}", 
             state.d_neo, state.d_turb_base, state.v_neo);
    println!("{}", "=".repeat(60));

    while state.time < t_max {
        state.update(dt);

        if step % 10000 == 0 {  // ⭐ 4000 → 10000
            println!(
                "t={:.2}s | n_Z(0)={:.2e} | Mode={:?}",
                state.time, state.impurity_density[0], state.confinement_mode
            );
        }
        step += 1;
    }

    println!("{}", "=".repeat(60));
    println!("📊 최종 통계:");
    println!("  중심 불순물: {:.2e} m⁻³", state.impurity_density[0]);
    println!("  가장자리 불순물: {:.2e} m⁻³", state.impurity_density[state.nr-1]);
    
    if let Err(e) = state.save_to_csv("w7x_simulation.csv") {
        eprintln!("❌ 저장 실패: {}", e);
    } else {
        println!("💾 저장 완료: w7x_simulation.csv");
    }
}
