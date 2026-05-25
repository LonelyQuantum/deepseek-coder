use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const UTF8_BYTE_ESTIMATOR: &str = "utf8_bytes";
pub const CALIBRATED_UTF8_ESTIMATOR: &str = "calibrated_utf8";

const PPM_SCALE: u64 = 1_000_000;

pub trait TokenEstimator {
    fn estimate(&self, text: &str) -> Result<u64, TokenEstimatorError>;
    fn report(&self) -> TokenEstimatorReport;

    fn calibrate(&mut self, _text: &str, _actual_tokens: u64) -> Result<(), TokenEstimatorError> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenEstimatorReport {
    pub name: String,
    pub exact: bool,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration: Option<TokenEstimatorCalibrationReport>,
}

impl TokenEstimatorReport {
    pub fn utf8_bytes() -> Self {
        Self {
            name: UTF8_BYTE_ESTIMATOR.to_owned(),
            exact: false,
            description:
                "UTF-8 byte count used as a deterministic proxy estimate; not a provider tokenizer."
                    .to_owned(),
            calibration: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenEstimatorCalibrationReport {
    pub sample_count: usize,
    pub input_unit: String,
    pub slope_ppm: u64,
    pub intercept_tokens: i64,
    pub mean_absolute_percentage_error_ppm: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEstimatorConfig {
    Utf8Bytes(Utf8BytesEstimator),
    Calibrated(CalibratedEstimator),
}

impl TokenEstimatorConfig {
    pub fn utf8_bytes() -> Self {
        Self::Utf8Bytes(Utf8BytesEstimator)
    }
}

impl Default for TokenEstimatorConfig {
    fn default() -> Self {
        Self::utf8_bytes()
    }
}

impl From<Utf8BytesEstimator> for TokenEstimatorConfig {
    fn from(estimator: Utf8BytesEstimator) -> Self {
        Self::Utf8Bytes(estimator)
    }
}

impl From<CalibratedEstimator> for TokenEstimatorConfig {
    fn from(estimator: CalibratedEstimator) -> Self {
        Self::Calibrated(estimator)
    }
}

impl TokenEstimator for TokenEstimatorConfig {
    fn estimate(&self, text: &str) -> Result<u64, TokenEstimatorError> {
        match self {
            Self::Utf8Bytes(estimator) => estimator.estimate(text),
            Self::Calibrated(estimator) => estimator.estimate(text),
        }
    }

    fn report(&self) -> TokenEstimatorReport {
        match self {
            Self::Utf8Bytes(estimator) => estimator.report(),
            Self::Calibrated(estimator) => estimator.report(),
        }
    }

    fn calibrate(&mut self, text: &str, actual_tokens: u64) -> Result<(), TokenEstimatorError> {
        match self {
            Self::Utf8Bytes(estimator) => estimator.calibrate(text, actual_tokens),
            Self::Calibrated(estimator) => estimator.calibrate(text, actual_tokens),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Utf8BytesEstimator;

impl TokenEstimator for Utf8BytesEstimator {
    fn estimate(&self, text: &str) -> Result<u64, TokenEstimatorError> {
        u64::try_from(text.len()).map_err(|_| TokenEstimatorError::TokenCountOverflow)
    }

    fn report(&self) -> TokenEstimatorReport {
        TokenEstimatorReport::utf8_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibratedEstimator {
    slope_ppm: u64,
    intercept_tokens: i64,
    mean_absolute_percentage_error_ppm: u64,
    sample_count: usize,
    samples: Vec<TokenCalibrationSample>,
}

impl CalibratedEstimator {
    pub fn from_samples(
        samples: impl IntoIterator<Item = TokenCalibrationSample>,
    ) -> Result<Self, TokenEstimatorError> {
        let samples = samples.into_iter().collect::<Vec<_>>();
        fit_calibration(samples)
    }

    pub fn from_coefficients(
        slope_ppm: u64,
        intercept_tokens: i64,
        mean_absolute_percentage_error_ppm: u64,
        sample_count: usize,
    ) -> Result<Self, TokenEstimatorError> {
        if slope_ppm == 0 || sample_count < 2 {
            return Err(TokenEstimatorError::InvalidCalibration);
        }

        Ok(Self {
            slope_ppm,
            intercept_tokens,
            mean_absolute_percentage_error_ppm,
            sample_count,
            samples: Vec::with_capacity(sample_count),
        })
    }

    pub const fn slope_ppm(&self) -> u64 {
        self.slope_ppm
    }

    pub const fn intercept_tokens(&self) -> i64 {
        self.intercept_tokens
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub const fn mean_absolute_percentage_error_ppm(&self) -> u64 {
        self.mean_absolute_percentage_error_ppm
    }

    fn estimate_from_units(&self, input_units: u64) -> Result<u64, TokenEstimatorError> {
        let scaled = u128::from(input_units)
            .checked_mul(u128::from(self.slope_ppm))
            .ok_or(TokenEstimatorError::TokenCountOverflow)?
            / u128::from(PPM_SCALE);
        let signed = i128::try_from(scaled).map_err(|_| TokenEstimatorError::TokenCountOverflow)?
            + i128::from(self.intercept_tokens);

        if signed <= 0 {
            return Ok(0);
        }

        u64::try_from(signed).map_err(|_| TokenEstimatorError::TokenCountOverflow)
    }
}

impl TokenEstimator for CalibratedEstimator {
    fn estimate(&self, text: &str) -> Result<u64, TokenEstimatorError> {
        let input_units =
            u64::try_from(text.len()).map_err(|_| TokenEstimatorError::TokenCountOverflow)?;
        self.estimate_from_units(input_units)
    }

    fn report(&self) -> TokenEstimatorReport {
        TokenEstimatorReport {
            name: CALIBRATED_UTF8_ESTIMATOR.to_owned(),
            exact: false,
            description:
                "Linear calibration over UTF-8 byte counts using provider usage samples; not a provider tokenizer."
                    .to_owned(),
            calibration: Some(TokenEstimatorCalibrationReport {
                sample_count: self.sample_count(),
                input_unit: UTF8_BYTE_ESTIMATOR.to_owned(),
                slope_ppm: self.slope_ppm,
                intercept_tokens: self.intercept_tokens,
                mean_absolute_percentage_error_ppm: self.mean_absolute_percentage_error_ppm,
            }),
        }
    }

    fn calibrate(&mut self, text: &str, actual_tokens: u64) -> Result<(), TokenEstimatorError> {
        let mut samples = self.samples.clone();
        samples.push(TokenCalibrationSample::from_text(text, actual_tokens)?);
        let updated = fit_calibration(samples)?;
        *self = updated;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenCalibrationSample {
    pub input_units: u64,
    pub actual_tokens: u64,
}

impl TokenCalibrationSample {
    pub fn new(input_units: u64, actual_tokens: u64) -> Result<Self, TokenEstimatorError> {
        if actual_tokens == 0 {
            return Err(TokenEstimatorError::InvalidActualTokens);
        }
        Ok(Self {
            input_units,
            actual_tokens,
        })
    }

    pub fn from_text(text: &str, actual_tokens: u64) -> Result<Self, TokenEstimatorError> {
        Self::new(
            u64::try_from(text.len()).map_err(|_| TokenEstimatorError::TokenCountOverflow)?,
            actual_tokens,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TokenEstimatorError {
    #[error("token count overflowed")]
    TokenCountOverflow,
    #[error("actual token count must be greater than zero")]
    InvalidActualTokens,
    #[error("calibration requires at least two samples, got {sample_count}")]
    InsufficientSamples { sample_count: usize },
    #[error("calibration samples must contain at least two different input sizes")]
    DegenerateSamples,
    #[error("calibration coefficients are invalid")]
    InvalidCalibration,
}

fn fit_calibration(
    samples: Vec<TokenCalibrationSample>,
) -> Result<CalibratedEstimator, TokenEstimatorError> {
    if samples.len() < 2 {
        return Err(TokenEstimatorError::InsufficientSamples {
            sample_count: samples.len(),
        });
    }
    if samples.iter().any(|sample| sample.actual_tokens == 0) {
        return Err(TokenEstimatorError::InvalidActualTokens);
    }

    let sample_count = samples.len() as f64;
    let sum_x = samples
        .iter()
        .map(|sample| sample.input_units as f64)
        .sum::<f64>();
    let sum_y = samples
        .iter()
        .map(|sample| sample.actual_tokens as f64)
        .sum::<f64>();
    let sum_xx = samples
        .iter()
        .map(|sample| {
            let x = sample.input_units as f64;
            x * x
        })
        .sum::<f64>();
    let sum_xy = samples
        .iter()
        .map(|sample| sample.input_units as f64 * sample.actual_tokens as f64)
        .sum::<f64>();
    let denominator = sample_count * sum_xx - sum_x * sum_x;
    if denominator.abs() < f64::EPSILON {
        return Err(TokenEstimatorError::DegenerateSamples);
    }

    let slope = (sample_count * sum_xy - sum_x * sum_y) / denominator;
    if !slope.is_finite() || slope <= 0.0 {
        return Err(TokenEstimatorError::InvalidCalibration);
    }
    let intercept = (sum_y - slope * sum_x) / sample_count;
    if !intercept.is_finite() {
        return Err(TokenEstimatorError::InvalidCalibration);
    }

    let slope_ppm = (slope * PPM_SCALE as f64).round();
    if !slope_ppm.is_finite() || slope_ppm <= 0.0 || slope_ppm > u64::MAX as f64 {
        return Err(TokenEstimatorError::InvalidCalibration);
    }
    let intercept_tokens = intercept.round();
    if !intercept_tokens.is_finite()
        || intercept_tokens < i64::MIN as f64
        || intercept_tokens > i64::MAX as f64
    {
        return Err(TokenEstimatorError::InvalidCalibration);
    }

    let mut estimator = CalibratedEstimator {
        slope_ppm: slope_ppm as u64,
        intercept_tokens: intercept_tokens as i64,
        mean_absolute_percentage_error_ppm: 0,
        sample_count: samples.len(),
        samples,
    };
    estimator.mean_absolute_percentage_error_ppm = mean_absolute_percentage_error_ppm(&estimator)?;

    Ok(estimator)
}

fn mean_absolute_percentage_error_ppm(
    estimator: &CalibratedEstimator,
) -> Result<u64, TokenEstimatorError> {
    let total = estimator
        .samples
        .iter()
        .map(|sample| {
            let predicted = estimator.estimate_from_units(sample.input_units)?;
            let actual = sample.actual_tokens;
            let absolute_error = predicted.abs_diff(actual);
            Ok((absolute_error as f64 / actual as f64) * PPM_SCALE as f64)
        })
        .collect::<Result<Vec<_>, TokenEstimatorError>>()?
        .into_iter()
        .sum::<f64>();
    let mean = total / estimator.samples.len() as f64;

    if !mean.is_finite() || mean < 0.0 || mean > u64::MAX as f64 {
        return Err(TokenEstimatorError::InvalidCalibration);
    }

    Ok(mean.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::{
        CalibratedEstimator, TokenCalibrationSample, TokenEstimator, TokenEstimatorConfig,
        TokenEstimatorError, Utf8BytesEstimator,
    };

    #[test]
    fn utf8_bytes_estimator_reports_deterministic_proxy_metadata() {
        let estimator = Utf8BytesEstimator;

        assert_eq!(estimator.estimate("hello").expect("estimate"), 5);
        let report = estimator.report();
        assert_eq!(report.name, "utf8_bytes");
        assert!(!report.exact);
        assert!(report.calibration.is_none());
    }

    #[test]
    fn calibrated_estimator_fits_linear_samples_without_storing_text() {
        let estimator = CalibratedEstimator::from_samples([
            TokenCalibrationSample::new(100, 50).expect("sample"),
            TokenCalibrationSample::new(200, 100).expect("sample"),
            TokenCalibrationSample::new(300, 150).expect("sample"),
        ])
        .expect("calibration should fit");

        assert_eq!(estimator.estimate_from_units(400).expect("estimate"), 200);
        assert_eq!(estimator.slope_ppm(), 500_000);
        assert_eq!(estimator.intercept_tokens(), 0);
        assert_eq!(estimator.sample_count(), 3);
        assert_eq!(estimator.mean_absolute_percentage_error_ppm(), 0);

        let report = estimator.report();
        assert_eq!(report.name, "calibrated_utf8");
        assert!(!report.exact);
        let calibration = report.calibration.expect("calibration metadata");
        assert_eq!(calibration.input_unit, "utf8_bytes");
        assert_eq!(calibration.sample_count, 3);
    }

    #[test]
    fn calibrated_estimator_rejects_degenerate_samples() {
        let error = CalibratedEstimator::from_samples([
            TokenCalibrationSample::new(100, 50).expect("sample"),
            TokenCalibrationSample::new(100, 60).expect("sample"),
        ])
        .expect_err("same input sizes cannot fit a line");

        assert_eq!(error, TokenEstimatorError::DegenerateSamples);
    }

    #[test]
    fn calibrated_estimator_reports_loaded_coefficient_sample_count() {
        let estimator =
            CalibratedEstimator::from_coefficients(500_000, 1, 25_000, 12).expect("coefficients");

        assert_eq!(estimator.sample_count(), 12);
        assert_eq!(estimator.estimate_from_units(100).expect("estimate"), 51);
        assert_eq!(
            estimator
                .report()
                .calibration
                .expect("calibration metadata")
                .sample_count,
            12
        );
    }

    #[test]
    fn token_estimator_config_delegates_to_selected_estimator() {
        let config = TokenEstimatorConfig::from(
            CalibratedEstimator::from_samples([
                TokenCalibrationSample::new(10, 5).expect("sample"),
                TokenCalibrationSample::new(20, 10).expect("sample"),
            ])
            .expect("calibration should fit"),
        );

        assert_eq!(config.estimate("1234567890").expect("estimate"), 5);
        assert_eq!(config.report().name, "calibrated_utf8");
    }
}
