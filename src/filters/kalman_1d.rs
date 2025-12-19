use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KalmanFilter1D {
    q: f64,
    r: f64,
    p: f64,
    x: f64,
    k: f64,
    initialized: bool,
    dead_zone: f64,
    last_output: f64,
}

impl KalmanFilter1D {
    pub fn new(q: f64, r: f64) -> Self {
        Self {
            q,
            r,
            p: r,
            x: 0.0,
            k: 0.0,
            initialized: false,
            dead_zone: 0.01,
            last_output: 0.0,
        }
    }

    // New: Allow configuring dead zone threshold
    pub fn with_dead_zone(mut self, threshold: f64) -> Self {
        self.dead_zone = threshold;
        self
    }

    pub fn update(&mut self, measurement: f64) -> f64 {
        if !self.initialized {
            self.x = measurement;
            self.last_output = measurement;
            self.initialized = true;
            return measurement;
        }

        // Kalman filter prediction and update
        self.p += self.q;
        self.k = self.p / (self.p + self.r);

        // Adaptive smoothing based on measurement delta
        let delta = (measurement - self.x).abs();
        let alpha = if delta > 1.0 {
            self.k * 1.5 // Faster response to large changes
        } else {
            self.k * 0.8 // More smoothing for small changes
        };

        // Update state estimate
        self.x += alpha * (measurement - self.x);
        self.p *= 1.0 - self.k;
        self.p = self.p.clamp(self.r * 0.1, self.r * 10.0);

        // Dead zone filter
        let change = (self.x - self.last_output).abs();
        let output = if change < self.dead_zone {
            self.last_output // No change if within dead zone
        } else {
            self.x // Use new value if change is significant
        };

        self.last_output = output;
        output
    }

    /// Reset the filter to initial state
    pub fn reset(&mut self) {
        self.x = 0.0;
        self.p = self.r;
        self.k = 0.0;
        self.initialized = false;
        self.last_output = 0.0;
    }

    /// Get the current state estimate
    pub fn get_estimate(&self) -> f64 {
        self.x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_measurement() {
        let mut filter = KalmanFilter1D::new(0.1, 0.1);
        let measurement = 10.0;
        let result = filter.update(measurement);
        assert_eq!(result, measurement);
    }

    #[test]
    fn test_noise_reduction() {
        let mut filter = KalmanFilter1D::new(0.1, 1.0);
        filter.update(10.0); // Initialize
        let noisy_measurement = 15.0;
        let filtered = filter.update(noisy_measurement);
        assert!(filtered > 10.0 && filtered < noisy_measurement);
    }

    #[test]
    fn test_reset() {
        let mut filter = KalmanFilter1D::new(0.1, 0.1);
        filter.update(10.0);
        filter.reset();
        assert_eq!(filter.get_estimate(), 0.0);
        assert!(!filter.initialized);
    }

    #[test]
    fn test_dead_zone() {
        let mut filter = KalmanFilter1D::new(0.1, 0.1).with_dead_zone(0.1);
        filter.update(1.0); // Initialize

        // Small change within dead zone
        let result = filter.update(1.05);
        assert_eq!(result, 1.0); // Should maintain previous value

        // Large change outside dead zone
        let result = filter.update(1.2);
        assert!(result > 1.0); // Should change
    }
}
