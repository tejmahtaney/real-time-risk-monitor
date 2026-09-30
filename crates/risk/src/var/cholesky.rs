//! A small hand-rolled Cholesky decomposition with shrinkage, over `Vec<Vec<f64>>`.
//!
//! `marketdata::simulator` already has a `cholesky_with_shrinkage` over
//! `nalgebra::DMatrix`, but `risk` cannot depend on `marketdata` (CLAUDE.md's
//! dependency direction: `risk` sits alongside `marketdata` under `engine`,
//! not above it), and pulling in `nalgebra` here just for matrices of a
//! handful of underlyings would be a heavyweight dependency for a few dozen
//! floating-point operations. This is a deliberate, documented duplication of
//! the same *algorithm* (shrink towards the identity in steps of 0.05 until
//! the matrix factorises) — the same trade-off
//! `engine/examples/tune_limits.rs` already makes for
//! `risk::monitor::next_state`. If `marketdata`'s version ever changes, this
//! one should be checked against it by hand.

/// Shrinkage increment: λ goes 0.05, 0.10, … until the Cholesky factor exists.
const SHRINKAGE_STEP: f64 = 0.05;

/// The lower Cholesky factor `L` of a symmetric positive-definite `matrix`
/// (`L · Lᵀ = matrix`), or `None` at the first non-positive pivot.
///
/// A plain O(n³) implementation: `matrix` holds at most a handful of
/// underlyings, so there is no need for anything more sophisticated.
fn cholesky(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = matrix.len();
    let mut l = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in 0..=i {
            // Reads two different rows (i and j) at the same index k, so this is a sum
            // over an index range rather than a single vector's own iterator.
            let sum: f64 = (0..j).map(|k| l[i][k] * l[j][k]).sum();
            if i == j {
                let diag = matrix[i][i] - sum;
                if !(diag > 0.0 && diag.is_finite()) {
                    return None;
                }
                l[i][j] = diag.sqrt();
            } else {
                if l[j][j] == 0.0 {
                    return None;
                }
                l[i][j] = (matrix[i][j] - sum) / l[j][j];
            }
        }
    }
    Some(l)
}

/// `(1 − λ) · matrix + λ · I`.
fn shrink_towards_identity(matrix: &[Vec<f64>], lambda: f64) -> Vec<Vec<f64>> {
    let n = matrix.len();
    (0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    let identity_ij = if i == j { 1.0 } else { 0.0 };
                    matrix[i][j] * (1.0 - lambda) + identity_ij * lambda
                })
                .collect()
        })
        .collect()
}

fn identity(n: usize) -> Vec<Vec<f64>> {
    (0..n)
        .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect()
}

/// As `marketdata::simulator::cholesky_with_shrinkage`: tries `matrix` as-is,
/// then shrinks it towards the identity in steps of 0.05 until it
/// factorises, returning the lower factor and the shrinkage weight λ used
/// (`0.0` if none was needed). `matrix` need not already be a valid
/// correlation matrix — an empty matrix (no underlyings) returns an empty
/// factor with `λ = 0.0`.
pub(super) fn cholesky_with_shrinkage(matrix: &[Vec<f64>]) -> (Vec<Vec<f64>>, f64) {
    let n = matrix.len();
    for step in 0..=20_u32 {
        let lambda = (f64::from(step) * SHRINKAGE_STEP).min(1.0);
        let shrunk = if step == 0 {
            matrix.to_vec()
        } else {
            shrink_towards_identity(matrix, lambda)
        };
        if let Some(factor) = cholesky(&shrunk) {
            return (factor, lambda);
        }
    }
    (identity(n), 1.0)
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    /// `L · Lᵀ`, for checking a factor against the matrix it came from.
    fn multiply_l_lt(l: &[Vec<f64>]) -> Vec<Vec<f64>> {
        let n = l.len();
        (0..n)
            .map(|i| {
                (0..n)
                    .map(|j| (0..n).map(|k| l[i][k] * l[j][k]).sum())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_known_positive_definite_matrix_factorises_exactly() {
        let matrix = vec![vec![4.0, 2.0], vec![2.0, 3.0]];
        let factor = cholesky(&matrix).expect("should factorise");
        assert_relative_eq!(factor[0][0], 2.0, epsilon = 1e-12);
        assert_relative_eq!(factor[1][0], 1.0, epsilon = 1e-12);
        assert_relative_eq!(factor[1][1], 2.0_f64.sqrt(), epsilon = 1e-12);
        assert_relative_eq!(factor[0][1], 0.0, epsilon = 1e-12);
    }

    // `lambda` is an exact sentinel (`step * 0.05` with `step = 0`, i.e. `0.0` exactly),
    // not a general float computation, so an exact comparison is correct here.
    #[allow(clippy::float_cmp)]
    #[test]
    fn identity_needs_no_shrinkage() {
        let id = identity(3);
        let (factor, lambda) = cholesky_with_shrinkage(&id);
        assert_eq!(lambda, 0.0);
        assert_eq!(factor, id);
    }

    #[test]
    fn a_non_positive_definite_matrix_is_shrunk_until_it_factorises() {
        // Schur complement of the second pivot: 0.5 - 1.0^2 / 1.0 = -0.5 < 0.
        let bad = vec![vec![1.0, 1.0], vec![1.0, 0.5]];
        assert!(cholesky(&bad).is_none(), "should not factorise as-is");

        let (factor, lambda) = cholesky_with_shrinkage(&bad);
        assert!(lambda > 0.0, "should have needed shrinkage");
        let reconstructed = multiply_l_lt(&factor);
        let shrunk = shrink_towards_identity(&bad, lambda);
        for i in 0..2 {
            for j in 0..2 {
                assert_relative_eq!(reconstructed[i][j], shrunk[i][j], epsilon = 1e-9);
            }
        }
    }

    // As above: `lambda` here is the same exact `0.0` sentinel.
    #[allow(clippy::float_cmp)]
    #[test]
    fn empty_matrix_factorises_to_empty_with_no_shrinkage() {
        let (factor, lambda) = cholesky_with_shrinkage(&[]);
        assert_eq!(lambda, 0.0);
        assert!(factor.is_empty());
    }
}
