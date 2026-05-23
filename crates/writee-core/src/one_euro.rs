//! 1€ filter — low-latency, low-jitter smoothing for pointer input.
//!
//! Reference: Casiez, Roussel, Vogel (CHI 2012). At rest the cutoff is
//! `mincutoff`; the cutoff rises with motion speed proportional to `beta`,
//! so fast strokes pass through with low lag while held-still tremors are
//! smoothed out.

#[derive(Debug, Clone, Copy)]
pub struct OneEuroParams {
    pub mincutoff: f32,
    pub beta: f32,
    pub dcutoff: f32,
}

impl Default for OneEuroParams {
    fn default() -> Self {
        // Sensible defaults for ink at ~120Hz. Tune per device.
        Self { mincutoff: 1.0, beta: 0.007, dcutoff: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct OneEuro {
    params: OneEuroParams,
    last_t_s: Option<f32>,
    last_x: Option<f32>,
    last_dx: f32,
}

impl OneEuro {
    pub fn new(params: OneEuroParams) -> Self {
        Self { params, last_t_s: None, last_x: None, last_dx: 0.0 }
    }

    pub fn reset(&mut self) {
        self.last_t_s = None;
        self.last_x = None;
        self.last_dx = 0.0;
    }

    pub fn filter(&mut self, t_s: f32, x: f32) -> f32 {
        let dt = match self.last_t_s {
            Some(prev) => (t_s - prev).max(1e-4),
            None => {
                self.last_t_s = Some(t_s);
                self.last_x = Some(x);
                return x;
            }
        };
        self.last_t_s = Some(t_s);
        let prev_x = self.last_x.unwrap_or(x);

        let dx = (x - prev_x) / dt;
        let edx = lowpass(dx, self.last_dx, alpha(dt, self.params.dcutoff));
        self.last_dx = edx;

        let cutoff = self.params.mincutoff + self.params.beta * edx.abs();
        let ex = lowpass(x, prev_x, alpha(dt, cutoff));
        self.last_x = Some(ex);
        ex
    }
}

fn alpha(dt: f32, cutoff: f32) -> f32 {
    let tau = 1.0 / (2.0 * std::f32::consts::PI * cutoff);
    1.0 / (1.0 + tau / dt)
}

fn lowpass(x: f32, prev: f32, a: f32) -> f32 {
    a * x + (1.0 - a) * prev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_first_sample_through() {
        let mut f = OneEuro::new(OneEuroParams::default());
        assert_eq!(f.filter(0.0, 7.5), 7.5);
    }

    #[test]
    fn smooths_noise_around_constant() {
        let mut f = OneEuro::new(OneEuroParams::default());
        let mut last = 0.0;
        for i in 0..200 {
            let t = i as f32 / 120.0;
            // 100 ± noise
            let noise = if i % 2 == 0 { 0.5 } else { -0.5 };
            last = f.filter(t, 100.0 + noise);
        }
        // After settling, the filtered value should be close to 100 even
        // though raw input keeps oscillating.
        assert!((last - 100.0).abs() < 0.3, "got {last}");
    }
}
