#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

use image::imageops::FilterType;
use tract_onnx::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartModel {
    Fast,
    Balanced,
    Best,
}

impl SmartModel {
    pub fn from_id(id: &str) -> Self {
        match id {
            "resnet18-v1-7" => Self::Fast,
            "resnet152-v1-7" => Self::Best,
            _ => Self::Balanced,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Fast => "resnet18-v1-7",
            Self::Balanced => "resnet50-v1-7",
            Self::Best => "resnet152-v1-7",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Fast => "Fast (ResNet-18)",
            Self::Balanced => "Balanced (ResNet-50)",
            Self::Best => "Best (ResNet-152)",
        }
    }

    pub fn filename(&self) -> &'static str {
        match self {
            Self::Fast => "resnet18-v1-7.onnx",
            Self::Balanced => "resnet50-v1-7.onnx",
            Self::Best => "resnet152-v1-7.onnx",
        }
    }

    pub fn url(&self) -> &'static str {
        match self {
            Self::Fast => "https://huggingface.co/onnxmodelzoo/resnet18-v1-7/resolve/main/resnet18-v1-7.onnx",
            Self::Balanced => "https://huggingface.co/onnxmodelzoo/resnet50-v1-7/resolve/main/resnet50-v1-7.onnx",
            Self::Best => "https://huggingface.co/onnxmodelzoo/resnet152-v1-7/resolve/main/resnet152-v1-7.onnx",
        }
    }

    pub fn sha256(&self) -> &'static str {
        match self {
            Self::Fast => "4e8f8653e7a2222b3904cc3fe8e304cd8b339ce1d05fd24688162f86fb6df52c",
            Self::Balanced => "361d7b3ea1e0d375355694a15993202e86c039fbcd4d1d86d9f783cb2bda247f",
            Self::Best => "f75ea2c6d4ba5372332d7ea8940828d546059239d5b7463f6834d8b584eb2e3a",
        }
    }
}

type Plan = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub struct LocalTagger {
    pub model_path: PathBuf,
    plan: OnceLock<Option<Plan>>,
}

impl LocalTagger {
    pub fn new(model_path: PathBuf) -> Self {
        Self {
            model_path,
            plan: OnceLock::new(),
        }
    }

    fn get_plan(&self) -> Option<&Plan> {
        self.plan
            .get_or_init(|| load_plan(&self.model_path).ok())
            .as_ref()
    }
}

fn load_plan(path: &PathBuf) -> TractResult<Plan> {
    tract_onnx::onnx()
        .model_for_path(path)?
        .with_input_fact(0, f32::fact([1usize, 3, 224, 224]).into())?
        .into_optimized()?
        .into_runnable()
}

impl LocalTagger {
    pub fn suggest_tags(&self, rgba: &[u8], width: u32, height: u32) -> Vec<String> {
        let Some(plan) = self.get_plan() else {
            return vec![];
        };

        let Some(img) = image::RgbaImage::from_raw(width, height, rgba.to_vec()) else {
            return vec![];
        };

        let resized = image::imageops::resize(&img, 224, 224, FilterType::Triangle);

        let means = [0.485f32, 0.456, 0.406];
        let stds = [0.229f32, 0.224, 0.225];
        let mut tensor = tract_ndarray::Array4::<f32>::zeros((1, 3, 224, 224));

        for y in 0..224usize {
            for x in 0..224usize {
                let px = resized.get_pixel(x as u32, y as u32);
                for c in 0..3usize {
                    tensor[[0, c, y, x]] = (px[c] as f32 / 255.0 - means[c]) / stds[c];
                }
            }
        }

        let input: Tensor = tensor.into();
        let Ok(outputs) = plan.run(tvec![input.into()]) else {
            return vec![];
        };
        let Ok(logits) = outputs[0].to_array_view::<f32>() else {
            return vec![];
        };

        let logits = logits.iter().copied().collect::<Vec<f32>>();
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let exps = logits
            .iter()
            .map(|&x| (x - max).exp())
            .collect::<Vec<f32>>();
        let sum: f32 = exps.iter().sum();
        if sum <= 0.0 {
            eprintln!("[smart] sum of exps is zero");
            return vec![];
        }
        let probs = exps.iter().map(|&e| e / sum).collect::<Vec<f32>>();

        let label_list = LABELS.lines().collect::<Vec<&str>>();
        let mut indexed = probs
            .iter()
            .copied()
            .enumerate()
            .collect::<Vec<(usize, f32)>>();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        indexed
            .iter()
            .take(5)
            .filter(|(_, p)| *p >= 0.05)
            .filter_map(|(i, _)| label_list.get(*i).map(|s| (*s).to_string()))
            .collect()
    }
}

static LABELS: &str = include_str!("imagenet_labels.txt");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_does_not_panic() {
        let rgba = vec![255u8; 224 * 224 * 4];
        let tagger = LocalTagger::new(PathBuf::from("/nonexistent/model.onnx"));
        let result = tagger.suggest_tags(&rgba, 224, 224);
        assert!(result.is_empty());
    }
}
