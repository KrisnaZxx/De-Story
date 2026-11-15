#[derive(Debug, Clone, Copy)]
pub struct Silence {
    pub start: f64,
    pub end: f64,
}

pub type SilenceInterval = Silence;

#[derive(Debug, Clone, Copy)]
struct Cand {
    t: f64,
    dur: f64,
}

pub fn choose_cuts(
    total: f64,
    weights: &[f64],
    silences: &[Silence],
    known: Option<&[f64]>,
) -> Vec<f64> {
    let n = weights.len();
    if n < 2 {
        return Vec::new();
    }

    let weights: Vec<f64> = weights.iter().map(|w| if w.is_finite() && *w > 0.0 { *w } else { 1.0 }).collect();
    let sum: f64 = weights.iter().sum();
    let sum = if sum > 0.0 { sum } else { n as f64 };

    let mut expected: Vec<f64> = Vec::new();
    if let Some(k) = known {
        if k.len() == n - 1 && k.iter().all(|v| v.is_finite()) && std::iter::once(0.0).chain(k.iter().copied()).zip(k.iter().copied().chain(std::iter::once(total))).enumerate().all(|(i,(a,b))| b - a >= (total * weights[i] / sum * 0.25).min(3.0)) {
            expected.extend_from_slice(k);
        }
    }
    if expected.is_empty() {
        let mut acc = 0.0;
        for k in 0..(n - 1) {
            let w = if weights[k] > 0.0 { weights[k] } else { 1.0 };
            acc += w;
            expected.push((total * acc) / sum);
        }
    }

    let avg = total / (n as f64);
    let min_gap = (avg * 0.3).min(3.0).max(0.05);

    let mut cands: Vec<Cand> = silences
        .iter()
        .filter(|s| s.start > 0.05 && s.end < total - 0.05)
        .map(|s| Cand {
            t: (s.start + s.end) / 2.0,
            dur: s.end - s.start,
        })
        .collect();

    for &t in &expected {
        cands.push(Cand { t, dur: 0.0 });
    }

    cands.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    let m = cands.len();

    let score = |k: usize, j: usize| -> f64 {
        let distance = (cands[j].t - expected[k]).abs();
        if distance > (avg * 0.15).min(0.75) { f64::NEG_INFINITY } else { cands[j].dur.min(0.4) - distance * 2.0 }
    };

    let mut best: Vec<Vec<f64>> = vec![vec![f64::NEG_INFINITY; m]; n - 1];
    let mut from: Vec<Vec<isize>> = vec![vec![-1; m]; n - 1];

    for j in 0..m {
        if cands[j].t >= min_gap {
            best[0][j] = score(0, j);
        }
    }

    for k in 1..(n - 1) {
        for j in 0..m {
            for i in 0..j {
                if best[k - 1][i] == f64::NEG_INFINITY || (cands[j].t - cands[i].t) < min_gap {
                    continue;
                }
                let v = best[k - 1][i] + score(k, j);
                if v > best[k][j] {
                    best[k][j] = v;
                    from[k][j] = i as isize;
                }
            }
        }
    }

    let mut best_j = -1isize;
    for x in 0..m {
        if total - cands[x].t >= min_gap {
            if best_j < 0 || best[n - 2][x] > best[n - 2][best_j as usize] {
                best_j = x as isize;
            }
        }
    }

    if best_j < 0 || best[n - 2][best_j as usize] == f64::NEG_INFINITY {
        return expected;
    }

    let mut cuts = Vec::new();
    let mut cur = best_j;
    for k in (0..=(n - 2)).rev() {
        cuts.push(cands[cur as usize].t);
        cur = from[k][cur as usize];
    }
    cuts.reverse();
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distant_silence_and_collapsed_known_cuts_cannot_destroy_a_scene() {
        let cuts = choose_cuts(30.0, &[10.0,10.0,10.0], &[Silence { start: 9.5, end: 18.0 }], Some(&[9.5,10.3]));
        assert_eq!(cuts, vec![10.0,20.0]);
        assert_eq!(choose_cuts(30.0, &[1.0,1.0,1.0], &[], Some(&[9.8,20.2])), vec![9.8,20.2]);
    }
}
