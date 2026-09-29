//! Numerical integrators.
//!
//! Two solvers are provided:
//!
//! * [`Rk4`], a fixed-step fourth-order Runge-Kutta. This is the transparent
//!   reference solver: its step size is exactly what the user typed, which makes
//!   it the right choice for reproducing a run.
//! * [`Rk45`], an adaptive Dormand-Prince 5(4) pair with embedded error
//!   estimate, step rejection, and a minimum step floor.
//!
//! Both operate on the flat 14-element state array from [`crate::state`]. Neither
//! normalises the quaternion: that is the stepper's job, so the integrator stays
//! a pure numerical method and the normalisation policy remains visible in the
//! run configuration.

use hex_core::{Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::state::{RigidBodyState, STATE_LEN};

/// The right-hand side of the equations of motion.
pub trait Derivative: Send + Sync {
    /// Fill `dx` with the derivative of `x` at time `t`.
    fn derivative(&self, t: Real, x: &[Real; STATE_LEN], dx: &mut [Real; STATE_LEN]);
}

/// Which integrator to use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "solver", rename_all = "snake_case")]
pub enum SolverKind {
    /// Fixed-step classical Runge-Kutta 4.
    Rk4 {
        /// Integration step in seconds.
        step: Real,
    },
    /// Adaptive Dormand-Prince 5(4).
    Rk45 {
        /// Relative tolerance on the scaled error norm.
        relative_tolerance: Real,
        /// Absolute tolerance on the scaled error norm.
        absolute_tolerance: Real,
        /// Largest step the solver may attempt, seconds.
        maximum_step: Real,
        /// Smallest step the solver may attempt, seconds.
        minimum_step: Real,
        /// Step used to start the integration, seconds.
        initial_step: Real,
    },
}

impl Default for SolverKind {
    fn default() -> Self {
        SolverKind::Rk4 { step: 0.001 }
    }
}

impl SolverKind {
    /// A fixed-step solver with the given step.
    ///
    /// A non-positive step is preserved rather than clamped, so validation can
    /// report it as a blocking problem instead of silently substituting a value
    /// the user did not ask for.
    pub fn fixed(step: Real) -> Self {
        SolverKind::Rk4 { step }
    }

    /// An adaptive solver with the given tolerances.
    pub fn adaptive(relative: Real, absolute: Real) -> Self {
        SolverKind::Rk45 {
            relative_tolerance: relative.max(1e-14),
            absolute_tolerance: absolute.max(1e-16),
            maximum_step: 0.1,
            minimum_step: 1e-9,
            initial_step: 1e-4,
        }
    }

    /// Human-readable name for the run header.
    pub fn label(&self) -> String {
        match *self {
            SolverKind::Rk4 { step } => format!("Fixed-step RK4, dt = {:.6} s", step),
            SolverKind::Rk45 {
                relative_tolerance,
                absolute_tolerance,
                ..
            } => format!(
                "Adaptive Dormand-Prince 5(4), rtol {:.1e}, atol {:.1e}",
                relative_tolerance, absolute_tolerance
            ),
        }
    }

    /// Shorter label for a status strip.
    pub fn short(&self) -> &'static str {
        match self {
            SolverKind::Rk4 { .. } => "RK4",
            SolverKind::Rk45 { .. } => "RK45",
        }
    }

    /// Whether the solver chooses its own step size.
    pub fn is_adaptive(&self) -> bool {
        matches!(self, SolverKind::Rk45 { .. })
    }

    /// One-line description stored in run metadata.
    pub fn describe(&self) -> String {
        self.label()
    }
}

/// Outcome of one attempted step.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    /// Time actually advanced.
    pub advanced: Real,
    /// Step size used.
    pub step_used: Real,
    /// Estimated local error norm, zero for a fixed-step solver.
    pub error_estimate: Real,
    /// Step size the adaptive solver suggests for the next attempt.
    pub suggested_next_step: Real,
    /// Whether the step was rejected and must be retried with a smaller step.
    pub rejected: bool,
}

/// Per-step statistics the run records and reports.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SolverStatistics {
    /// Number of steps accepted.
    pub accepted_steps: u64,
    /// Number of steps rejected by the adaptive error test.
    pub rejected_steps: u64,
    /// Number of right-hand-side evaluations.
    pub function_evaluations: u64,
    /// Smallest accepted step.
    pub minimum_step_used: Real,
    /// Largest accepted step.
    pub maximum_step_used: Real,
}

impl SolverStatistics {
    /// Fraction of attempted steps that were rejected.
    pub fn rejection_ratio(&self) -> Real {
        let attempts = self.accepted_steps + self.rejected_steps;
        if attempts == 0 {
            0.0
        } else {
            self.rejected_steps as Real / attempts as Real
        }
    }

    /// True when the solver struggled, which usually means the step is too large
    /// for the stiffness present.
    pub fn is_struggling(&self) -> bool {
        self.rejected_steps > 0 && self.rejection_ratio() > 0.1
    }
}

/// Fixed-step classical Runge-Kutta 4.
///
/// The four stages are the textbook ones. Mass is advanced with the same rule as
/// every other component, so a variable-mass run integrates `dm/dt` at the same
/// order of accuracy.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rk4;

impl Rk4 {
    /// Advance the state by one step.
    pub fn step<D: Derivative + ?Sized>(
        &self,
        dynamics: &D,
        t: Real,
        x: &[Real; STATE_LEN],
        h: Real,
        out: &mut [Real; STATE_LEN],
    ) -> StepOutcome {
        let mut k1 = [0.0; STATE_LEN];
        let mut k2 = [0.0; STATE_LEN];
        let mut k3 = [0.0; STATE_LEN];
        let mut k4 = [0.0; STATE_LEN];
        let mut tmp = [0.0; STATE_LEN];

        dynamics.derivative(t, x, &mut k1);

        for i in 0..STATE_LEN {
            tmp[i] = x[i] + 0.5 * h * k1[i];
        }
        dynamics.derivative(t + 0.5 * h, &tmp, &mut k2);

        for i in 0..STATE_LEN {
            tmp[i] = x[i] + 0.5 * h * k2[i];
        }
        dynamics.derivative(t + 0.5 * h, &tmp, &mut k3);

        for i in 0..STATE_LEN {
            tmp[i] = x[i] + h * k3[i];
        }
        dynamics.derivative(t + h, &tmp, &mut k4);

        for i in 0..STATE_LEN {
            out[i] = x[i] + (h / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
        }

        StepOutcome {
            advanced: h,
            step_used: h,
            error_estimate: 0.0,
            suggested_next_step: h,
            rejected: false,
        }
    }
}

/// Which components participate in the adaptive error norm.
///
/// Including the quaternion in the error norm is correct but scales oddly
/// because the four components are not independent. The default weights the
/// quaternion at half strength, which keeps the controller responsive to
/// attitude without letting a redundant component dominate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ErrorWeights {
    pub position: Real,
    pub velocity: Real,
    pub quaternion: Real,
    pub angular_velocity: Real,
    pub mass: Real,
}

impl Default for ErrorWeights {
    fn default() -> Self {
        Self {
            position: 1.0,
            velocity: 1.0,
            quaternion: 0.5,
            angular_velocity: 1.0,
            mass: 1.0,
        }
    }
}

/// Adaptive Dormand-Prince 5(4).
#[derive(Debug, Clone, Copy, Default)]
pub struct Rk45;

impl Rk45 {
    /// Dormand-Prince coefficients for the fifth-order solution.
    const A: [[Real; 6]; 7] = [
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [1.0 / 5.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [3.0 / 40.0, 9.0 / 40.0, 0.0, 0.0, 0.0, 0.0],
        [44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0, 0.0, 0.0, 0.0],
        [
            19372.0 / 6561.0,
            -25360.0 / 2187.0,
            64448.0 / 6561.0,
            -212.0 / 729.0,
            0.0,
            0.0,
        ],
        [
            9017.0 / 3168.0,
            -355.0 / 33.0,
            46732.0 / 5247.0,
            49.0 / 176.0,
            -5103.0 / 18656.0,
            0.0,
        ],
        [
            35.0 / 384.0,
            0.0,
            500.0 / 1113.0,
            125.0 / 192.0,
            -2187.0 / 6784.0,
            11.0 / 84.0,
        ],
    ];

    /// Fifth-order weights, which are also the coefficients of the FSAL stage.
    const B5: [Real; 7] = [
        35.0 / 384.0,
        0.0,
        500.0 / 1113.0,
        125.0 / 192.0,
        -2187.0 / 6784.0,
        11.0 / 84.0,
        0.0,
    ];

    /// Fourth-order weights used only for the error estimate.
    const B4: [Real; 7] = [
        5179.0 / 57600.0,
        0.0,
        7571.0 / 16695.0,
        393.0 / 640.0,
        -92097.0 / 339200.0,
        187.0 / 2100.0,
        1.0 / 40.0,
    ];

    /// Attempt one adaptive step.
    ///
    /// On rejection `out` is left untouched and the caller must retry with
    /// `suggested_next_step`.
    #[allow(clippy::too_many_arguments)]
    pub fn step<D: Derivative + ?Sized>(
        &self,
        dynamics: &D,
        t: Real,
        x: &[Real; STATE_LEN],
        h: Real,
        weights: &ErrorWeights,
        relative_tolerance: Real,
        absolute_tolerance: Real,
        out: &mut [Real; STATE_LEN],
    ) -> StepOutcome {
        let c: [Real; 7] = [0.0, 1.0 / 5.0, 3.0 / 10.0, 4.0 / 5.0, 8.0 / 9.0, 1.0, 1.0];
        let mut k = [[0.0; STATE_LEN]; 7];

        for stage in 0..7 {
            // The first stage is the derivative at the current state; the last
            // stage is reused from the fifth-order result because the method is
            // first-same-as-last.
            if stage == 0 {
                dynamics.derivative(t, x, &mut k[0]);
            } else if stage == 6 {
                // k[6] is evaluated at the fifth-order solution below.
            } else {
                let mut tmp = *x;
                for (j, kj) in k.iter().enumerate().take(stage) {
                    let a = Self::A[stage][j];
                    if a == 0.0 {
                        continue;
                    }
                    for i in 0..STATE_LEN {
                        tmp[i] += h * a * kj[i];
                    }
                }
                dynamics.derivative(t + c[stage] * h, &tmp, &mut k[stage]);
            }

            if stage == 5 {
                // Build the fifth-order candidate so the final stage can be
                // evaluated at it, completing the FSAL pair.
                let mut y5 = *x;
                for (j, kj) in k.iter().enumerate().take(6) {
                    let b = Self::B5[j];
                    if b == 0.0 {
                        continue;
                    }
                    for i in 0..STATE_LEN {
                        y5[i] += h * b * kj[i];
                    }
                }
                dynamics.derivative(t + h, &y5, &mut k[6]);
            }
        }

        // Fifth-order solution and the embedded fourth-order error estimate.
        let mut y5 = *x;
        let mut y4 = *x;
        for (j, kj) in k.iter().enumerate() {
            let b5 = Self::B5[j];
            let b4 = Self::B4[j];
            for i in 0..STATE_LEN {
                y5[i] += h * b5 * kj[i];
                y4[i] += h * b4 * kj[i];
            }
        }

        // Scaled RMS error norm.
        let mut sum = 0.0;
        for i in 0..STATE_LEN {
            let weight = weights.component_weight(i);
            let scale = absolute_tolerance + relative_tolerance * x[i].abs().max(y5[i].abs());
            let e = (y5[i] - y4[i]) / scale.max(1e-300);
            sum += weight * e * e;
        }
        let error = (sum / STATE_LEN as Real).sqrt();

        if error <= 1.0 || !error.is_finite() {
            *out = y5;
            // Standard PI-free step controller with a safety factor.
            let factor = if error <= 1e-16 {
                5.0
            } else {
                0.9 * error.powf(-0.2)
            };
            StepOutcome {
                advanced: h,
                step_used: h,
                error_estimate: error,
                suggested_next_step: h * factor.clamp(0.2, 5.0),
                rejected: false,
            }
        } else {
            let factor = (0.9 * error.powf(-0.25)).clamp(0.1, 1.0);
            StepOutcome {
                advanced: 0.0,
                step_used: h,
                error_estimate: error,
                suggested_next_step: h * factor,
                rejected: true,
            }
        }
    }
}

impl ErrorWeights {
    fn component_weight(&self, index: usize) -> Real {
        match index {
            0..=2 => self.position,
            3..=5 => self.velocity,
            6..=9 => self.quaternion,
            10..=12 => self.angular_velocity,
            _ => self.mass,
        }
    }
}

/// Normalisation policy for the attitude quaternion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NormalizationPolicy {
    /// Normalise after every accepted step. The default and recommended choice.
    #[default]
    EveryStep,
    /// Normalise only when the norm error exceeds a threshold.
    Threshold {
        /// Error above which normalisation is applied.
        maximum_error: Real,
    },
    /// Never normalise. Only for diagnosing drift deliberately.
    Never,
}

impl NormalizationPolicy {
    pub fn label(&self) -> String {
        match *self {
            NormalizationPolicy::EveryStep => "Normalize every step".to_string(),
            NormalizationPolicy::Threshold { maximum_error } => {
                format!("Normalize above {:.1e} norm error", maximum_error)
            }
            NormalizationPolicy::Never => "Never normalize (diagnostic)".to_string(),
        }
    }

    /// Apply the policy to a state, returning whether normalisation happened.
    pub fn apply(&self, state: &mut RigidBodyState) -> bool {
        match *self {
            NormalizationPolicy::EveryStep => {
                state.normalize_attitude();
                true
            }
            NormalizationPolicy::Threshold { maximum_error } => {
                if state.quaternion_norm_error() > maximum_error {
                    state.normalize_attitude();
                    true
                } else {
                    false
                }
            }
            NormalizationPolicy::Never => false,
        }
    }
}

/// Combine the flat array back into a state, applying the mass policy.
pub fn state_from_array(x: &[Real; STATE_LEN], constant_mass: Option<Real>) -> RigidBodyState {
    let mut s = RigidBodyState::from_array(x);
    if let Some(m) = constant_mass {
        s.mass = m;
    }
    s
}

/// Write a state into the flat array, honouring a constant mass declaration.
pub fn state_to_array(state: &RigidBodyState, constant_mass: Option<Real>) -> [Real; STATE_LEN] {
    let mut x = state.to_array();
    if let Some(m) = constant_mass {
        x[13] = m;
    }
    x
}

/// Convenience: the world-frame acceleration implied by a force and mass.
pub fn acceleration_from_force(force_world: Vec3, mass: Real) -> Vec3 {
    if mass.abs() < 1e-12 {
        Vec3::zeros()
    } else {
        force_world / mass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A linear test system: dv/dt = constant, dr/dt = v. Lets the integrator be
    /// checked against an exact answer.
    struct ConstantAcceleration {
        acceleration: Vec3,
    }

    impl Derivative for ConstantAcceleration {
        fn derivative(&self, _t: Real, x: &[Real; STATE_LEN], dx: &mut [Real; STATE_LEN]) {
            dx[0] = x[3];
            dx[1] = x[4];
            dx[2] = x[5];
            dx[3] = self.acceleration.x;
            dx[4] = self.acceleration.y;
            dx[5] = self.acceleration.z;
            for d in dx.iter_mut().skip(6) {
                *d = 0.0;
            }
        }
    }

    /// dx/dt = x, whose exact solution is exp(t).
    struct ExponentialGrowth;

    impl Derivative for ExponentialGrowth {
        fn derivative(&self, _t: Real, x: &[Real; STATE_LEN], dx: &mut [Real; STATE_LEN]) {
            dx.copy_from_slice(x);
        }
    }

    fn zero_state() -> [Real; STATE_LEN] {
        [0.0; STATE_LEN]
    }

    #[test]
    fn rk4_is_exact_for_constant_acceleration() {
        let d = ConstantAcceleration {
            acceleration: Vec3::new(0.0, 0.0, -9.81),
        };
        let mut x = zero_state();
        x[6] = 1.0; // identity quaternion
        x[3] = 10.0; // initial horizontal velocity
        x[13] = 1.0; // mass

        let h = 0.01;
        let steps = 100;
        let mut t = 0.0;
        for _ in 0..steps {
            let mut out = [0.0; STATE_LEN];
            Rk4.step(&d, t, &x, h, &mut out);
            x = out;
            t += h;
        }

        // Position is quadratic in time, so RK4 is exact for it.
        let expected_z = -0.5 * 9.81 * t * t;
        let expected_x = 10.0 * t;
        assert!((x[0] - expected_x).abs() < 1e-9, "x = {}", x[0]);
        assert!(
            (x[2] - expected_z).abs() < 1e-9,
            "z = {} vs {}",
            x[2],
            expected_z
        );
        assert!((x[5] + 9.81 * t).abs() < 1e-9);
    }

    #[test]
    fn rk4_fourth_order_convergence_on_exponential() {
        let d = ExponentialGrowth;
        let mut errors = Vec::new();
        for h in [0.1 as Real, 0.05, 0.025, 0.0125] {
            let mut x = [1.0; STATE_LEN];
            let steps = (1.0 as Real / h).round() as usize;
            let mut t = 0.0;
            for _ in 0..steps {
                let mut out = [0.0; STATE_LEN];
                Rk4.step(&d, t, &x, h, &mut out);
                x = out;
                t += h;
            }
            let exact = t.exp();
            errors.push((x[0] - exact).abs());
        }
        // Each halving of h should cut the error by about 16.
        for w in errors.windows(2) {
            let ratio = w[0] / w[1];
            assert!(
                ratio > 8.0,
                "convergence ratio {} is not fourth order",
                ratio
            );
        }
    }

    #[test]
    fn rk45_matches_rk4_on_a_smooth_problem() {
        let d = ConstantAcceleration {
            acceleration: Vec3::new(0.5, -0.25, -9.81),
        };
        let mut x = zero_state();
        x[6] = 1.0;
        x[3] = 5.0;
        x[13] = 1.0;

        let weights = ErrorWeights::default();
        let mut t = 0.0;
        let mut h: Real = 0.01;
        let mut x45 = x;
        while t < 1.0 {
            let step = h.min(1.0 - t);
            let mut out = [0.0; STATE_LEN];
            let outcome = Rk45.step(&d, t, &x45, step, &weights, 1e-8, 1e-10, &mut out);
            if !outcome.rejected {
                x45 = out;
                t += outcome.advanced;
            }
            h = outcome.suggested_next_step.clamp(1e-6, 0.05);
        }

        let mut x4 = x;
        let mut t4 = 0.0;
        while t4 < 1.0 - 1e-12 {
            let mut out = [0.0; STATE_LEN];
            Rk4.step(&d, t4, &x4, 1e-4, &mut out);
            x4 = out;
            t4 += 1e-4;
        }

        for i in 0..STATE_LEN {
            assert!(
                (x45[i] - x4[i]).abs() < 1e-6,
                "component {} differs: {} vs {}",
                i,
                x45[i],
                x4[i]
            );
        }
    }

    #[test]
    fn rk45_rejects_a_step_that_is_too_large() {
        let d = ExponentialGrowth;
        let x = [1.0; STATE_LEN];
        let weights = ErrorWeights::default();
        let mut out = [0.0; STATE_LEN];
        // A very large step against a tight tolerance must be rejected.
        let outcome = Rk45.step(&d, 0.0, &x, 10.0, &weights, 1e-12, 1e-14, &mut out);
        assert!(outcome.rejected);
        assert!(outcome.suggested_next_step < 10.0);
        assert!(outcome.error_estimate > 1.0);
    }

    #[test]
    fn rk45_accepts_a_small_step() {
        let d = ExponentialGrowth;
        let x = [1.0; STATE_LEN];
        let weights = ErrorWeights::default();
        let mut out = [0.0; STATE_LEN];
        let outcome = Rk45.step(&d, 0.0, &x, 1e-8, &weights, 1e-8, 1e-10, &mut out);
        assert!(!outcome.rejected);
        assert!(outcome.error_estimate < 1.0);
        assert!(outcome.suggested_next_step >= outcome.step_used);
    }

    #[test]
    fn rk45_adaptive_step_stays_within_tolerance() {
        let d = ExponentialGrowth;
        let weights = ErrorWeights::default();
        let mut x = [1.0; STATE_LEN];
        let mut t = 0.0;
        let mut h: Real = 0.01;
        let mut rejected = 0;
        while t < 5.0 {
            let step = h.min(5.0 - t);
            let mut out = [0.0; STATE_LEN];
            let outcome = Rk45.step(&d, t, &x, step, &weights, 1e-9, 1e-12, &mut out);
            if outcome.rejected {
                rejected += 1;
                h = outcome.suggested_next_step;
                continue;
            }
            x = out;
            t += outcome.advanced;
            h = outcome.suggested_next_step.clamp(1e-10, 1.0);
        }
        let exact = 5.0f64.exp();
        let relative = (x[0] - exact).abs() / exact;
        assert!(relative < 1e-7, "relative error {}", relative);
        assert!(rejected < 20, "too many rejections: {}", rejected);
    }

    #[test]
    fn statistics_track_rejections() {
        let mut s = SolverStatistics {
            accepted_steps: 90,
            rejected_steps: 10,
            ..SolverStatistics::default()
        };
        assert!((s.rejection_ratio() - 0.1).abs() < 1e-12);
        assert!(!s.is_struggling());
        s.rejected_steps = 40;
        assert!(s.is_struggling());
    }

    #[test]
    fn normalization_policy_every_step_always_normalises() {
        let mut state = RigidBodyState {
            attitude: hex_core::Quaternion::new(2.0, 0.0, 0.0, 0.0),
            ..Default::default()
        };
        assert!(NormalizationPolicy::EveryStep.apply(&mut state));
        assert!(state.quaternion_norm_error() < 1e-15);
    }

    #[test]
    fn normalization_policy_threshold_only_acts_when_needed() {
        let policy = NormalizationPolicy::Threshold {
            maximum_error: 1e-6,
        };
        let mut good = RigidBodyState::default();
        assert!(!policy.apply(&mut good));

        let mut bad = RigidBodyState {
            attitude: hex_core::Quaternion::new(1.001, 0.0, 0.0, 0.0),
            ..Default::default()
        };
        assert!(policy.apply(&mut bad));
        assert!(bad.quaternion_norm_error() < 1e-15);
    }

    #[test]
    fn normalization_policy_never_leaves_the_quaternion_alone() {
        let policy = NormalizationPolicy::Never;
        let mut state = RigidBodyState {
            attitude: hex_core::Quaternion::new(1.5, 0.0, 0.0, 0.0),
            ..Default::default()
        };
        assert!(!policy.apply(&mut state));
        assert!((state.attitude.norm() - 1.5).abs() < 1e-12);
    }

    #[test]
    fn solver_kind_labels_and_flags() {
        let fixed = SolverKind::fixed(0.001);
        assert!(!fixed.is_adaptive());
        assert!(fixed.label().contains("0.001000"));
        assert_eq!(fixed.short(), "RK4");

        let adaptive = SolverKind::adaptive(1e-8, 1e-10);
        assert!(adaptive.is_adaptive());
        assert_eq!(adaptive.short(), "RK45");
    }

    #[test]
    fn constant_mass_overrides_the_mass_component() {
        let state = RigidBodyState {
            mass: 99.0,
            ..RigidBodyState::default()
        };
        let x = state_to_array(&state, Some(2.5));
        assert!((x[13] - 2.5).abs() < 1e-15);
        let back = state_from_array(&x, Some(2.5));
        assert!((back.mass - 2.5).abs() < 1e-15);

        let free = state_to_array(&state, None);
        assert!((free[13] - 99.0).abs() < 1e-15);
    }

    #[test]
    fn acceleration_from_force_handles_zero_mass() {
        assert!(acceleration_from_force(Vec3::new(1.0, 0.0, 0.0), 0.0).norm() < 1e-15);
        assert!(
            (acceleration_from_force(Vec3::new(10.0, 0.0, 0.0), 2.0) - Vec3::new(5.0, 0.0, 0.0))
                .norm()
                < 1e-15
        );
    }

    #[test]
    fn error_weights_assign_each_state_block() {
        let w = ErrorWeights::default();
        assert!((w.component_weight(0) - 1.0).abs() < 1e-15);
        assert!((w.component_weight(7) - 0.5).abs() < 1e-15);
        assert!((w.component_weight(11) - 1.0).abs() < 1e-15);
        assert!((w.component_weight(13) - 1.0).abs() < 1e-15);
    }
}
