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
    /// Creates a new KalmanFilter1D instance
    ///
    /// # Arguments
    /// * `q` - Process noise variance (how much we expect the true value to change between measurements)
    /// * `r` - Measurement noise variance (how noisy our measurements are)
    pub fn new(q: f64, r: f64) -> Self {
        Self {
            q,
            r,
            p: 1.0,
            x: 0.0,
            k: 0.0,
            initialized: false,
        }
    }

    /// Updates the filter with a new measurement
    ///
    /// # Arguments
    /// * `measurement` - The new measured value
    ///
    /// # Returns
    /// The filtered estimate after incorporating the new measurement
    pub fn update(&mut self, measurement: f64) -> f64 {
        if !self.initialized {
            self.x = measurement;
            self.initialized = true;
            return measurement;
        }

        // Prediction step
        self.p = self.p + self.q;

        // Update step
        self.k = self.p / (self.p + self.r);
        self.x = self.x + self.k * (measurement - self.x);
        self.p = (1.0 - self.k) * self.p;

        self.x
    }

    /// Resets the filter to its initial state
    pub fn reset(&mut self) {
        self.p = 1.0;
        self.x = 0.0;
        self.k = 0.0;
        self.initialized = false;
    }

    /// Gets the current state estimate without updating
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
}
