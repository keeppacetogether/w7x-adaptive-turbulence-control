use ndarray::{Array1, s};

#[derive(Clone)]
struct StellaratorState {
    // 공간 그리드
    radius_grid: Array1<f64>,           // 정규화 반경 [0, 1]
    dr: f64,                             // 그리드 간격
    nr: usize,                           // 그리드 개수
    
    // 물리량
    impurity_density: Array1<f64>,       // 불순물 밀도 n_Z(r) [m^-3]
    electron_density: Array1<f64>,       // 전자 밀도 n_e(r) [m^-3]
    electron_temp: Array1<f64>,          // 전자 온도 T_e(r) [keV]
    
    // 수송 계수
    d_neo: f64,                          // 신고전 확산 [m^2/s]
    d_turb_base: f64,                    // 기본 난류 확산 [m^2/s]
    v_neo: f64,                          // 신고전 대류 [m/s] (음수 = 내부)
    
    // 제어 상태
    confinement_mode: ConfinementMode,
    time: f64,                           // 현재 시간 [s]
    pulse_start_time: Option<f64>,       // 펄스 시작 시간
    
    // 진단 데이터
    center_impurity_history: Vec<f64>,
    edge_impurity_history: Vec<f64>,
    turbulence_history: Vec<f64>,
    time_history: Vec<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum ConfinementMode {
    Normal,           // 정상 운전 (난류 억제됨)
    TurbulencePulse,  // 난류 펄스 (조증 상태)
}

impl StellaratorState {
    fn new(nr: usize) -> Self {
        let dr = 1.0 / (nr - 1) as f64;
        let radius_grid = Array1::linspace(0.0, 1.0, nr);
        
        // 초기 프로파일 설정 (W7-X 스타일)
        let mut state = StellaratorState {
            radius_grid: radius_grid.clone(),
            dr,
            nr,
            impurity_density: Array1::zeros(nr),
            electron_density: Array1::zeros(nr),
            electron_temp: Array1::zeros(nr),
            d_neo: 0.1,           // m^2/s
            d_turb_base: 10.0,    // m^2/s
            v_neo: -1.0,          // m/s (내부 방향)
            confinement_mode: ConfinementMode::Normal,
            time: 0.0,
            pulse_start_time: None,
            center_impurity_history: Vec::new(),
            edge_impurity_history: Vec::new(),
            turbulence_history: Vec::new(),
            time_history: Vec::new(),
        };
        
        // 프로파일 초기화
        state.initialize_profiles();
        state
    }
    
    fn initialize_profiles(&mut self) {
        for (i, &r) in self.radius_grid.iter().enumerate() {
            // 중심 피킹 프로파일 (페라볼릭)
            self.electron_density[i] = 8e19 * (1.0 - r.powi(2));  // m^-3
            self.electron_temp[i] = 8.0 * (1.0 - r.powi(2));      // keV
            
            // 초기 불순물: 가장자리에 약간, 중심은 적음
            self.impurity_density[i] = 1e18 * (0.1 + 0.9 * r.powi(2));
        }
    }
    
    fn calculate_turbulence_level(&self, r_idx: usize) -> f64 {
        // ITG 난류 모델: 밀도/온도 기울기에 의존
        
        let r = self.radius_grid[r_idx];
        if r < 0.01 || r > 0.99 {
            return 0.1; // 경계에서는 난류 약함
        }
        
        // 기울기 계산 (중심 차분)
        let dn_dr = (self.electron_density[r_idx + 1] - 
                     self.electron_density[r_idx - 1]) / (2.0 * self.dr);
        let dT_dr = (self.electron_temp[r_idx + 1] - 
                     self.electron_temp[r_idx - 1]) / (2.0 * self.dr);
        
        // 기울기 길이
        let Ln = -self.electron_density[r_idx] / dn_dr.max(-1e20);
        let LT = -self.electron_temp[r_idx] / dT_dr.max(-1e20);
        
        // ITG 불안정성: LT/Ln이 작을수록 난류 강함
        let ratio = (LT / Ln).abs().max(0.1).min(10.0);
        
        // 난류 레벨 (경험적 모델)
        let turbulence_factor = match self.confinement_mode {
            ConfinementMode::Normal => {
                // 정상: 밀도/온도 프로파일이 비슷하면 난류 억제
                if ratio > 0.8 && ratio < 1.2 {
                    0.2  // 강하게 억제 (펠릿 주입 후 상태)
                } else {
                    1.0  // 보통
                }
            }
            ConfinementMode::TurbulencePulse => {
                // 펄스: 가장자리만 증폭
                if r > 0.7 {  // 가장자리 (r > 0.7)
                    3.0  // 3배 증가!
                } else {  // 중심은 보호
                    1.0
                }
            }
        };
        
        self.d_turb_base * turbulence_factor
    }
    
    fn calculate_flux(&self, r_idx: usize) -> f64 {
        // 불순물 플럭스: Γ = v_neo * n_Z - (D_neo + D_turb) * dn_Z/dr
        
        if r_idx == 0 || r_idx >= self.nr - 1 {
            return 0.0;  // 경계 조건
        }
        
        let n_Z = self.impurity_density[r_idx];
        let dn_Z_dr = (self.impurity_density[r_idx + 1] - 
                       self.impurity_density[r_idx - 1]) / (2.0 * self.dr);
        
        let D_turb = self.calculate_turbulence_level(r_idx);
        let D_total = self.d_neo + D_turb;
        
        // 대류 + 확산
        let flux_convection = self.v_neo * n_Z;
        let flux_diffusion = -D_total * dn_Z_dr;
        
        flux_convection + flux_diffusion
    }
    
    fn detect_impurity_accumulation(&self) -> bool {
        // AI 센서: 중심 불순물이 증가하고 있는가?
        
        // 1. 중심 불순물 농도 체크
        let center_impurity = self.impurity_density[0];
        let critical_density = 2e18;  // 임계값 [m^-3]
        
        if center_impurity > critical_density {
            return true;
        }
        
        // 2. 증가율 체크 (최근 이력)
        if self.center_impurity_history.len() > 10 {
            let recent = self.center_impurity_history.len() - 1;
            let old = self.center_impurity_history.len() - 10;
            let rate = (self.center_impurity_history[recent] - 
                       self.center_impurity_history[old]) / 
                       (self.time_history[recent] - self.time_history[old]);
            
            // 빠르게 증가 중이면
            if rate > 5e17 {  // [m^-3/s]
                return true;
            }
        }
        
        // 3. 가장자리 난류 레벨 체크
        let edge_turbulence = self.calculate_turbulence_level(self.nr - 2);
        if edge_turbulence < 2.0 {  // 너무 낮으면
            return true;
        }
        
        false
    }
    
    fn update(&mut self, dt: f64) {
        // 1. 제어 결정
        self.control_decision();
        
        // 2. 수송 방정식 풀기
        self.solve_transport_equation(dt);
        
        // 3. 진단 데이터 저장
        self.save_diagnostics();
        
        // 4. 시간 증가
        self.time += dt;
    }
    
    fn control_decision(&mut self) {
        match self.confinement_mode {
            ConfinementMode::Normal => {
                // 불순물 축적 감지
                if self.detect_impurity_accumulation() {
                    println!("⚠️  t={:.3}s: 불순물 축적 감지! 난류 펄스 시작", self.time);
                    self.confinement_mode = ConfinementMode::TurbulencePulse;
                    self.pulse_start_time = Some(self.time);
                }
            }
            ConfinementMode::TurbulencePulse => {
                // 100ms 후 자동 복귀
                if let Some(start) = self.pulse_start_time {
                    if self.time - start > 0.1 {  // 100ms
                        println!("✅ t={:.3}s: 난류 펄스 종료, 정상 모드 복귀", self.time);
                        self.confinement_mode = ConfinementMode::Normal;
                        self.pulse_start_time = None;
                    }
                }
            }
        }
    }
    
    fn solve_transport_equation(&mut self, dt: f64) {
        // ∂n_Z/∂t = -1/r * ∂(r*Γ)/∂r + S
        // Forward Euler (간단하지만 안정성 주의)
        
        let mut new_density = self.impurity_density.clone();
        
        for i in 1..self.nr-1 {
            let r = self.radius_grid[i];
            
            // 플럭스 계산
            let flux_r_plus = self.calculate_flux(i);
            let flux_r_minus = self.calculate_flux(i - 1);
            
            // 발산 계산: ∇·Γ ≈ (r*Γ|_{i+1/2} - r*Γ|_{i-1/2}) / (r * dr)
            let r_plus = self.radius_grid[i] + 0.5 * self.dr;
            let r_minus = self.radius_grid[i] - 0.5 * self.dr;
            
            let div_flux = if r > 0.01 {
                (r_plus * flux_r_plus - r_minus * flux_r_minus) / (r * self.dr)
            } else {
                // 중심 근처: L'Hôpital
                (flux_r_plus - flux_r_minus) / self.dr
            };
            
            // 소스 (가장자리에서 약간)
            let source = if r > 0.8 {
                1e17  // m^-3/s
            } else {
                0.0
            };
            
            // 업데이트
            let dn_dt = -div_flux + source;
            new_density[i] = self.impurity_density[i] + dn_dt * dt;
            
            // 음수 방지
            new_density[i] = new_density[i].max(0.0);
        }
        
        // 경계 조건
        new_density[0] = new_density[1];  // 중심: 대칭
        new_density[self.nr - 1] = 0.5 * new_density[self.nr - 2];  // 가장자리: 감소
        
        self.impurity_density = new_density;
    }
    
    fn save_diagnostics(&mut self) {
        self.center_impurity_history.push(self.impurity_density[0]);
        self.edge_impurity_history.push(self.impurity_density[self.nr - 1]);
        self.turbulence_history.push(self.calculate_turbulence_level(self.nr - 2));
        self.time_history.push(self.time);
    }
    
    fn print_status(&self) {
        let mode_str = match self.confinement_mode {
            ConfinementMode::Normal => "😌 정상",
            ConfinementMode::TurbulencePulse => "🔥 펄스",
        };
        
        println!("t={:.3}s | {} | n_Z(0)={:.2e} | n_Z(edge)={:.2e} | D_turb(edge)={:.1}",
                 self.time,
                 mode_str,
                 self.impurity_density[0],
                 self.impurity_density[self.nr - 1],
                 self.calculate_turbulence_level(self.nr - 2));
    }
}

// 메인 시뮬레이션
fn main() {
    println!("🌟 W7-X 적응형 난류 제어 시뮬레이션");
    println!("=" .repeat(60));
    
    let mut state = StellaratorState::new(101);  // 101 그리드 포인트
    
    let dt = 0.001;  // 1ms 타임스텝
    let t_max = 20.0;  // 20초 시뮬레이션
    
    let mut step = 0;
    while state.time < t_max {
        state.update(dt);
        
        // 100ms마다 출력
        if step % 100 == 0 {
            state.print_status();
        }
        
        step += 1;
    }
    
    println!("\n" + &"=".repeat(60));
    println!("✅ 시뮬레이션 완료!");
    
    // 결과 분석
    analyze_results(&state);
    
    // CSV 저장 (Python 플롯용)
    save_to_csv(&state);
}

fn analyze_results(state: &StellaratorState) {
    println!("\n📊 결과 분석:");
    
    let interventions = state.time_history.windows(2)
        .zip(state.turbulence_history.windows(2))
        .filter(|(_, turb)| turb[1] > turb[0] * 2.0)  // 2배 이상 증가
        .count();
    
    println!("  - 총 개입 횟수: {}", interventions);
    println!("  - 최종 중심 불순물: {:.2e} m^-3", 
             state.impurity_density[0]);
    println!("  - 초기 대비 변화: {:.1}%", 
             (state.impurity_density[0] / state.center_impurity_history[0] - 1.0) * 100.0);
}

fn save_to_csv(state: &StellaratorState) {
    use std::fs::File;
    use std::io::Write;
    
    let mut file = File::create("w7x_simulation.csv").unwrap();
    writeln!(file, "time,center_impurity,edge_impurity,turbulence").unwrap();
    
    for i in 0..state.time_history.len() {
        writeln!(file, "{},{},{},{}",
                 state.time_history[i],
                 state.center_impurity_history[i],
                 state.edge_impurity_history[i],
                 state.turbulence_history[i]).unwrap();
    }
    
    println!("\n💾 데이터 저장: w7x_simulation.csv");
}
