use serde::{Deserialize, Serialize};

/// Represents a basic Kalman filter for 1-dimensional data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KalmanFilter1D {
    q: f64,            // Process noise variance
    r: f64,            // Measurement noise variance
    p: f64,            // Estimation error covariance
    x: f64,            // State estimate
    k: f64,            // Kalman gain
    initialized: bool, // Tracks if filter has received first measurement
}

impl KalmanFilter1D {
    pub fn new(q: f64, r: f64) -> Self {
        Self {
            q,    // Process noise (lower = smoother, but less responsive)
            r,    // Measurement noise (higher = more smoothing)
            p: r, // Initialize P with R for better initial convergence
            x: 0.0,
            k: 0.0,
            initialized: false,
        }
    }

    pub fn update(&mut self, measurement: f64) -> f64 {
        if !self.initialized {
            self.x = measurement;
            self.initialized = true;
            return measurement;
        }

        // Prediction step - simplified for better performance
        self.p += self.q;

        // Update step with smoothing optimizations
        self.k = self.p / (self.p + self.r);

        // Adaptive smoothing based on measurement delta
        let delta = (measurement - self.x).abs();
        let alpha = if delta > 1.0 {
            // Faster response to large changes
            self.k * 1.5
        } else {
            // More smoothing for small changes
            self.k * 0.8
        };

        self.x += alpha * (measurement - self.x);
        self.p *= 1.0 - self.k;

        // Ensure P stays within reasonable bounds
        self.p = self.p.clamp(self.r * 0.1, self.r * 10.0);

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
}
