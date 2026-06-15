//! Gauss-Seidel solver for the PDN conductance system.
//!
//! The reduced free-node system is symmetric positive-definite and diagonally
//! dominant (each diagonal is the sum of incident conductances, ≥ the off-
//! diagonal sum, strictly so for any node touching a pad), so Gauss-Seidel
//! converges. Row `k` encodes `diag[k]·x[k] = rhs[k] + Σ (g·x[j])` over its
//! free neighbours — i.e. `x[k] ← (rhs[k] + Σ g·x[j]) / diag[k]`.
//!
//! Pure std — unit-tested on small networks with closed-form answers.

#[derive(Debug)]
pub struct LinSys {
    pub n: usize,
    pub diag: Vec<f64>,
    pub offdiag: Vec<Vec<(usize, f64)>>, // (neighbour, conductance)
    pub rhs: Vec<f64>,
}

#[derive(Debug)]
pub enum SolveError {
    Singular(usize),       // node index with zero diagonal (floating)
    NotConverged(f64),     // residual after the iteration cap
}

impl std::fmt::Display for SolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveError::Singular(k) => {
                write!(f, "singular PDN: node index {k} has no resistive path (floating)")
            }
            SolveError::NotConverged(r) => write!(f, "solver did not converge (residual {r:.3e})"),
        }
    }
}
impl std::error::Error for SolveError {}

impl LinSys {
    pub fn new(n: usize) -> LinSys {
        LinSys { n, diag: vec![0.0; n], offdiag: vec![Vec::new(); n], rhs: vec![0.0; n] }
    }

    /// Solve via Gauss-Seidel. `tol` is the max per-node update; `max_iter` caps work.
    pub fn solve(&self, max_iter: usize, tol: f64) -> Result<Vec<f64>, SolveError> {
        for k in 0..self.n {
            if self.diag[k] == 0.0 {
                return Err(SolveError::Singular(k));
            }
        }
        let mut x = vec![0.0f64; self.n];
        let mut last_delta = f64::INFINITY;
        for _ in 0..max_iter {
            let mut delta = 0.0f64;
            for k in 0..self.n {
                let mut acc = self.rhs[k];
                for &(j, g) in &self.offdiag[k] {
                    acc += g * x[j];
                }
                let xk = acc / self.diag[k];
                delta = delta.max((xk - x[k]).abs());
                x[k] = xk;
            }
            last_delta = delta;
            if delta < tol {
                return Ok(x);
            }
        }
        Err(SolveError::NotConverged(last_delta))
    }
}
