// SPDX-License-Identifier: GPL-3.0-or-later
//! A Gaussian Mixture Model for background subtraction of thermal images.
//!
//! # Citations
//! Zivkovic, Z., & van der Heijden, F. (2006). Efficient adaptive density estimation per image
//! pixel for the task of background subtraction. In Pattern Recognition Letters (Vol. 27, Issue 7,
//! pp. 773–780). Elsevier BV. https://doi.org/10.1016/j.patrec.2005.11.005
use std::iter;

use bitvec::prelude::*;
use rayon::prelude::*;
use serde::Deserialize;

use super::learning_rate::LearningRate;

/// A single gaussian distribution.
///
/// Each pixel's model can be made up of multiple distributions, and the number of distributions in
/// each model can vary of the lifetime of the model.
#[derive(Clone, Debug, PartialEq)]
struct GaussianComponent {
    /// The *squared* variance.
    variance: f32,
    /// The mean of this distribution.
    mean: f32,
    /// The amount that this distribution contributes to the final output of the model.
    weight: f32,
}

impl GaussianComponent {
    fn new(sample: f32, learning_rate: f32, variance: f32) -> Self {
        Self {
            variance,
            mean: sample,
            weight: learning_rate,
        }
    }

    fn probability_density(&self, sample: f32) -> f32 {
        // Univariate gaussian pdf
        let numer = -0.5 * ((sample - self.mean).powi(2) / self.variance);
        let denom = (self.variance * 2.0 * std::f32::consts::PI).sqrt();
        numer.exp() / denom
    }

    /// Compute the squared Mahalanobis distance for a given value and this distribution.
    fn squared_mahalanobis(&self, sample: f32) -> f32 {
        (sample - self.mean).powi(2) / self.variance
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct GmmParameters {
    /// The rate at which new values are incorporated into the model.
    ///
    /// In the paper, this value is referred to as $\alpha$. It is more efficient to vary this
    /// value during the training of a fresh model (by starting from 1, and decreasing until the
    /// final learning rate is reached), but it doesn't seriously impact performance.
    #[serde(default = "GmmParameters::default_learning_rate")]
    pub(crate) learning_rate: LearningRate,

    /// A hard limit on the number of gaussians used to model each pixel.
    ///
    /// This value doesn't normally need to be changed from the default.
    #[serde(default = "GmmParameters::default_max_components")]
    pub(crate) max_components: usize,

    /// The distance below which a sample is considered "close" to a specific distribution.
    ///
    /// This value is *squared*, so a distance of 4 would actually be 16 (`4 * 4`).
    #[serde(default = "GmmParameters::default_model_distance_threshold")]
    pub(crate) model_distance_threshold: f32,

    /// A negative pressure applied when updating weights.
    ///
    /// When a component's weight drops below 0, the component is removed.
    #[serde(default = "GmmParameters::default_complexity_reduction")]
    pub(crate) complexity_reduction: f32,

    /// The threshold above which a distribution (or group of distributions) is considered part of
    /// the background model.
    ///
    /// This is referred to as $1 - c_f$ in the papers.
    #[serde(default = "GmmParameters::default_background_threshold")]
    pub(crate) background_threshold: f32,

    /// The initial variance for newly created distributions added to a model.
    #[serde(default = "GmmParameters::default_initial_variance")]
    pub(crate) initial_variance: f32,
}

impl GmmParameters {
    // These defaults are all functions so that they can be used with the serde default annotation.

    /// Corresponds to looking at the last 500 samples.
    const fn default_learning_rate() -> LearningRate {
        LearningRate::new(0.002)
    }

    /// The original paper used 4, which seems like a good enough reason to continue using it.
    const fn default_max_components() -> usize {
        4
    }

    const fn default_model_distance_threshold() -> f32 {
        9.0
    }

    const fn default_complexity_reduction() -> f32 {
        0.05
    }

    const fn default_background_threshold() -> f32 {
        0.01
    }

    const fn default_initial_variance() -> f32 {
        10.0
    }
}

impl Default for GmmParameters {
    fn default() -> Self {
        Self {
            learning_rate: Self::default_learning_rate(),
            max_components: Self::default_max_components(),
            model_distance_threshold: Self::default_model_distance_threshold(),
            complexity_reduction: Self::default_complexity_reduction(),
            background_threshold: Self::default_background_threshold(),
            initial_variance: Self::default_initial_variance(),
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(super) struct GaussianMixtureModel(Vec<GaussianComponent>);

// As a precondition to this implementation, `components` *must* always be sorted by component
// weight.
impl GaussianMixtureModel {
    fn evaluate(&self, sample: f32) -> f32 {
        self.0
            .iter()
            .map(|model| model.weight * model.probability_density(sample))
            .sum()
    }

    fn insert(&mut self, component: GaussianComponent) {
        if self.0.is_empty() {
            self.0.push(component)
        } else {
            let target_weight = component.weight;
            let index = self.0.partition_point(|c| target_weight < c.weight);
            self.0.insert(index, component);
        }
    }

    /// Update the background model with a new sample.
    ///
    /// `learning_rate` ($\alpha$ in the papers) is a value between 0 and 1 and describes the
    /// weight given to the new sample compared to the existing model. If not given, the inverse of
    /// the number of samples is used.
    fn update(&mut self, sample: f32, params: &GmmParameters) {
        let learning_rate = params.learning_rate.current_value();
        let complexity_reduction = learning_rate * params.complexity_reduction;
        let mut claimed = false;
        // Iterating over the *indices*, as we might be reordering the items (and changing the
        // slice while we're iterating over it is screwy). Also not using a for loop over a range
        // as we might be removing components as well.
        let mut index = 0;
        while index < self.0.len() {
            let component = &mut self.0[index];
            // A component claims (in the paper, "owns") a sample if it is close enough
            // (Mahalanobis distance above a threshold) and has the largest weight. Since the
            // components are sorted by weight, the first one that satisfies the first condition
            // claims the sample.
            let distance = component.squared_mahalanobis(sample);
            // The claiming component gets weight, mean and variance updated, but the other
            // components only have their weight updated.
            if !claimed && distance <= params.model_distance_threshold {
                claimed = true;
                component.weight += learning_rate * (1.0 - component.weight) - complexity_reduction;
                let difference = sample - component.mean;
                let weighted_learning_rate = learning_rate / component.weight;
                component.mean += weighted_learning_rate * difference;
                // The equation in the paper has (in pseudo-tex, and skipping the hats):
                // $\delta^{T}_{m} \delta_{m} - \sigma^{2}_{m}$
                // Because the values here are scalars, simple multiplication is used instead of
                // transposing and multiplying.
                component.variance +=
                    weighted_learning_rate * (difference.powi(2) - component.variance);
                // Drop `component` while keeping a copy of the weight around as we're about to
                // reorder the vector of components by weight next.
                let weight = component.weight;
                drop(component);
                // This component's weight has increased, so move it up in the list so that the
                // list is still sorted.
                let mut prev_index = index.saturating_sub(1);
                let mut current_index = index;
                while current_index != 0 && self.0[prev_index].weight < weight {
                    self.0.swap(current_index, prev_index);
                    current_index = prev_index;
                    prev_index = prev_index.saturating_sub(1);
                }
            } else {
                component.weight += learning_rate * (-component.weight) - complexity_reduction;
                // No need to sort after this, as all non-claiming components are reduced
                // proportionally.
            }
            // Remove negative components
            if self.0[index].weight < 0.0 {
                self.0.remove(index);
            } else {
                index += 1;
            }
        }
        // If no component "claims" a sample, add a new component
        if !claimed {
            // If adding a component would put us over the limit, drop the smallest component
            if self.0.len() + 1 > params.max_components {
                self.0.pop();
            }
            let new_component =
                GaussianComponent::new(sample, learning_rate, params.initial_variance);
            self.insert(new_component);
        }
        debug_assert_eq!(
            self.0,
            {
                let mut cloned = self.0.clone();
                cloned.sort_by(|a, b| a.weight.partial_cmp(&b.weight).unwrap().reverse());
                cloned
            },
            "GMM component list is not sorted by weight: {:#?}",
            self.0
        );
        // Normalize the weights so that they sum to 1.0
        let weights_sum: f32 = self.0.iter().map(|c| c.weight).sum();
        self.0
            // `par_iter_mut()`, or `iter_mut()`?
            .par_iter_mut()
            .for_each(|component| {
                component.weight /= weights_sum;
            });
    }

    fn background_probability(&self, sample: f32, params: &GmmParameters) -> f32 {
        let mut weight_sum = 0.0;
        let mut bg_probability = 0.0;
        for component in &self.0 {
            weight_sum += component.weight;
            bg_probability += component.weight * component.probability_density(sample);
            if weight_sum > params.background_threshold {
                break;
            }
        }
        bg_probability
    }
}

#[derive(Clone, Debug)]
pub(super) struct BackgroundModel<Container> {
    /// The model for each individual pixel.
    pixel_models: Container,
    /// The parameters shared by all models.
    parameters: GmmParameters,
    /// Flags corresponding to pixels that are *not* to be updated.
    // Using a BitArray would be more precise, but it isn't possible to perform operations on const
    // parameters.
    frozen_pixels: BitVec,
}

impl<Container> BackgroundModel<Container> {
    pub(super) fn set_parameters(&mut self, params: GmmParameters) {
        self.parameters = params;
    }

    #[inline]
    fn set_all_state(&mut self, state: bool) {
        self.frozen_pixels.set_all(state)
    }

    pub(super) fn thaw_all(&mut self) {
        self.set_all_state(false);
    }

    // TODO: Add optimized method for sorted pixels
    #[inline]
    fn set_pixel_state(&mut self, pixels: &[usize], state: bool) {
        for pixel_index in pixels {
            self.frozen_pixels.set(*pixel_index, state);
        }
    }

    pub(super) fn freeze_pixels(&mut self, pixels: &[usize]) {
        self.set_pixel_state(pixels, true)
    }

    pub(super) fn thaw_pixels(&mut self, pixels: &[usize]) {
        self.set_pixel_state(pixels, false)
    }
}

impl BackgroundModel<Vec<GaussianMixtureModel>> {
    pub(super) fn update(&mut self, samples: &[f32]) {
        let params = &self.parameters;
        let frozen = self.frozen_pixels.as_bitslice();
        samples
            .par_iter()
            .zip(self.pixel_models.par_iter_mut())
            .enumerate()
            .for_each(|(index, (sample, model))| {
                if !frozen[index] {
                    model.update(*sample, params)
                }
            });
        self.parameters.learning_rate.increment()
    }

    pub(super) fn background_probability<R>(&self, samples: &[f32]) -> R
    where
        R: FromParallelIterator<f32>,
    {
        let params = &self.parameters;
        samples
            .par_iter()
            .zip(self.pixel_models.par_iter())
            .map(|(sample, model)| model.background_probability(*sample, params))
            .collect()
    }
}

impl<Container> BackgroundModel<Container>
where
    Container: iter::FromIterator<GaussianMixtureModel>,
{
    pub(super) fn new(num_pixels: usize) -> Self {
        let pixel_models = (0..num_pixels).map(|_| Default::default()).collect();
        let frozen_pixels = iter::repeat(false).take(num_pixels).collect();
        Self {
            pixel_models,
            parameters: GmmParameters::default(),
            frozen_pixels,
        }
    }
}

#[cfg(test)]
mod test {
    use rand_chacha::ChaCha8Rng;
    use rand_core::SeedableRng;
    use rand_distr::{DistIter, Distribution, Normal};

    use super::{BackgroundModel, GaussianMixtureModel};

    type NormalSamples = DistIter<Normal<f32>, ChaCha8Rng, f32>;
    type GmmBackground = BackgroundModel<Vec<GaussianMixtureModel>>;

    const LENGTH: usize = 10;
    const TRAINING_SIZE: usize = 5000;

    fn rng() -> ChaCha8Rng {
        // Using a seeded, RNG so that tests are repeatable.
        // Seed generated with python3 -c 'import random; print(repr(random.randbytes(32)))'
        const SEED: &[u8; 32] = b"\
        \x91\xa0\x98\x81\x81y\xe1\x00\xc7\xd1\xd9*\xda\
        \xe4c\xbbJX|\xc5\xdb4z\x91\x0b\x10=}\xe5\xc9tm";
        ChaCha8Rng::from_seed(*SEED)
    }

    fn generate_image(samples: &mut NormalSamples, length: usize) -> Vec<f32> {
        samples.take(length).collect()
    }

    fn random_samples() -> (NormalSamples, NormalSamples) {
        const STDDEV: f32 = 1.0;
        const BG_MEAN: f32 = 22.0;
        const FG_MEAN: f32 = 37.0;
        (
            Normal::new(BG_MEAN, STDDEV).unwrap().sample_iter(rng()),
            Normal::new(FG_MEAN, STDDEV).unwrap().sample_iter(rng()),
        )
    }

    fn check_model(
        model: GmmBackground,
        bg_samples: &mut NormalSamples,
        fg_samples: &mut NormalSamples,
    ) {
        const LOCATIONS: [usize; 2] = [0, 8];
        // Get a new sample, but replace two locations with foreground values
        let mut testing_sample = generate_image(bg_samples, LENGTH);
        for location in &LOCATIONS {
            testing_sample[*location] = fg_samples.next().unwrap()
        }
        let classified: Vec<f32> = model.background_probability(&testing_sample);
        let threshold = 0.001;
        println!("Classified:\n{:#?}", classified);
        for (index, p) in classified.iter().enumerate() {
            if LOCATIONS.contains(&index) {
                assert!(
                    *p < threshold,
                    "Foreground probability ({}) too low for index {}",
                    1.0 - p,
                    index
                )
            } else {
                assert!(
                    *p >= threshold,
                    "Background probability ({}) too low for index {}",
                    p,
                    index
                )
            }
        }
    }

    // A very simple test; train only with samples distributed around room temperature, then add
    // two pixels that are closer to body temperature.
    #[test]
    fn simple() {
        let (mut bg_samples, mut fg_samples) = random_samples();
        let mut model: GmmBackground = BackgroundModel::new(LENGTH);
        for _ in 0..TRAINING_SIZE {
            let samples = generate_image(&mut bg_samples, LENGTH);
            model.update(&samples)
        }
        check_model(model, &mut bg_samples, &mut fg_samples);
    }

    // Test that after the "background" abruptly changes, the model (eventually) recalibrates to
    // the new background.
    #[test]
    fn abrupt_change() {
        let (mut bg_samples, mut fg_samples) = random_samples();
        let mut model: GmmBackground = BackgroundModel::new(LENGTH);
        // Start with the foreground being used for the background
        for _ in 0..TRAINING_SIZE {
            let samples = generate_image(&mut fg_samples, LENGTH);
            model.update(&samples)
        }
        // Then train again, but with the background values being the lower temperatures
        for _ in 0..TRAINING_SIZE {
            let samples = generate_image(&mut bg_samples, LENGTH);
            model.update(&samples)
        }
        // Then test
        check_model(model, &mut bg_samples, &mut &mut fg_samples);
    }

    // Test that pixels can be frozen at a value, and are *not* updated with the rest of the pixels.
    #[test]
    fn frozen_as_foreground() {
        let (mut bg_samples, mut fg_samples) = random_samples();
        let mut model: GmmBackground = BackgroundModel::new(LENGTH);
        // Start with a completely room temperature "image"
        for _ in 0..TRAINING_SIZE {
            let samples = generate_image(&mut bg_samples, LENGTH);
            model.update(&samples)
        }
        // Freeze two pixels, then update these pixels with body temperature values while
        // continuing training.
        const FROZEN_PIXELS: [usize; 2] = [4, 5];
        model.freeze_pixels(&FROZEN_PIXELS);
        for _ in 0..TRAINING_SIZE {
            let mut samples = generate_image(&mut bg_samples, LENGTH);
            for frozen in &FROZEN_PIXELS {
                samples[*frozen] = fg_samples.next().unwrap();
            }
            model.update(&samples)
        }
        // Then check. If the frozen pixels were ignored, the models for those pixels will still be
        // modeling the room temperature pixels and be treated as the background.
        check_model(model, &mut bg_samples, &mut &mut fg_samples);
    }
}
