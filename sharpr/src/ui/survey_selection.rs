//! Survey changes only which members of a selected set survive the review.
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct SurveySelection {
    paths: Vec<PathBuf>,
    dismissed: Vec<usize>,
}

impl SurveySelection {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            dismissed: Vec::new(),
        }
    }
    pub fn candidates(&self) -> Vec<(usize, &Path)> {
        self.paths
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.dismissed.contains(i))
            .map(|(i, path)| (i, path.as_path()))
            .collect()
    }
    pub fn dismiss(&mut self, number: usize) -> bool {
        if number >= self.paths.len() || self.dismissed.contains(&number) {
            return false;
        }
        self.dismissed.push(number);
        true
    }
    pub fn undo(&mut self) -> Option<usize> {
        self.dismissed.pop()
    }
    pub fn can_undo(&self) -> bool {
        !self.dismissed.is_empty()
    }
    pub fn survivors(&self) -> Vec<PathBuf> {
        self.candidates()
            .into_iter()
            .map(|(_, p)| p.to_path_buf())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arbitrary_dismissals_and_undo_keep_original_order_even_after_emptying() {
        let paths: Vec<_> = ["a", "b", "c"].map(PathBuf::from).into();
        let mut survey = SurveySelection::new(paths.clone());
        assert!(survey.dismiss(1));
        assert!(!survey.dismiss(1));
        assert!(!survey.dismiss(9));
        assert_eq!(survey.survivors(), vec![paths[0].clone(), paths[2].clone()]);
        assert!(survey.dismiss(0));
        assert!(survey.dismiss(2));
        assert!(survey.survivors().is_empty());
        assert_eq!(survey.undo(), Some(2));
        assert_eq!(survey.undo(), Some(0));
        assert_eq!(survey.undo(), Some(1));
        assert_eq!(survey.survivors(), paths);
        assert!(!survey.can_undo());
        assert_eq!(survey.undo(), None);
    }
}
