import pandas as pd

import matplotlib.pyplot as plt

import numpy as np



df = pd.read_csv('w7x_simulation.csv')



fig, axes = plt.subplots(3, 1, figsize=(14, 10))



# 1. 중심 불순물

axes[0].plot(df['time'], df['center_impurity']/1e18, 'b-', linewidth=2, label='Center')

axes[0].axhline(y=2.2, color='r', linestyle='--', alpha=0.5, label='Threshold')

axes[0].set_ylabel('Center n_Z\n(10¹⁸ m⁻³)', fontsize=12)

axes[0].legend(loc='upper right')

axes[0].grid(True, alpha=0.3)

axes[0].set_title('W7-X Adaptive Turbulence Control Simulation',

			   fontsize=14, fontweight='bold')



# 2. 가장자리 불순물

axes[1].plot(df['time'], df['edge_impurity']/1e18, 'r-', linewidth=2, label='Edge')

axes[1].set_ylabel('Edge n_Z\n(10¹⁸ m⁻³)', fontsize=12)

axes[1].legend(loc='upper right')

axes[1].grid(True, alpha=0.3)



# 3. 난류 레벨 + 펄스 구간 표시

axes[2].plot(df['time'], df['turbulence'], 'g-', linewidth=2, label='Edge Turbulence')

axes[2].axhline(y=4.0, color='gray', linestyle='--', alpha=0.5, label='Baseline')



# 펄스 구간 찾기 (turbulence > 10)

pulse_active = df['turbulence'] > 10

pulse_changes = pulse_active.astype(int).diff()

pulse_starts = df['time'][pulse_changes == 1].values

pulse_ends = df['time'][pulse_changes == -1].values



# 펄스 횟수

n_pulses = len(pulse_starts)

print(f"📊 총 {n_pulses}회 개입")



# 펄스 구간 색칠

for start, end in zip(pulse_starts, pulse_ends):

for ax in axes:

	ax.axvspan(start, end, alpha=0.2, color='yellow')



axes[2].set_ylabel('Turbulence\n(m²/s)', fontsize=12)

axes[2].set_xlabel('Time (s)', fontsize=12)

axes[2].legend(loc='upper right')

axes[2].grid(True, alpha=0.3)



plt.tight_layout()

plt.savefig('w7x_control_results_optimized.png', dpi=300, bbox_inches='tight')

print("💾 저장 완료: w7x_control_results_optimized.png")

plt.show()



# 통계

print(f"\n📈 최종 통계:")

print(f"  - 중심 불순물: {df['center_impurity'].iloc[-1]:.2e} m⁻³")

print(f"  - 초기 대비: {df['center_impurity'].iloc[-1]/df['center_impurity'].iloc[0]:.2f}x")

print(f"  - 평균 펄스 간격: {(10.0)/n_pulses:.2f}s")
