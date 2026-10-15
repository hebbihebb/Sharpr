//! Survey changes only which members of a selected set survive the review.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct SurveySelection {
    paths: Vec<PathBuf>,
    dismissed: Vec<usize>,
    retired: HashSet<usize>,
}

impl SurveySelection {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            dismissed: Vec::new(),
            retired: HashSet::new(),
        }
    }
    pub fn candidates(&self) -> Vec<(usize, &Path)> {
        self.paths
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.retired.contains(i))
            .map(|(i, path)| (i, path.as_path()))
            .collect()
    }
    pub fn dismiss(&mut self, number: usize) -> bool {
        if number >= self.paths.len() || !self.retired.insert(number) {
            return false;
        }
        self.dismissed.push(number);
        true
    }
    pub fn undo(&mut self) -> Option<usize> {
        let number = self.dismissed.pop()?;
        self.retired.remove(&number);
        Some(number)
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
    #[test]
    fn a_large_cull_keeps_survivors_ordered_and_restores_every_dismissal() {
        let paths: Vec<_> = (0..5000)
            .map(|i| PathBuf::from(format!("{i}.png")))
            .collect();
        let mut survey = SurveySelection::new(paths.clone());
        for number in (0..5000).step_by(2) {
            assert!(survey.dismiss(number));
        }
        assert_eq!(
            survey.survivors(),
            paths.iter().skip(1).step_by(2).cloned().collect::<Vec<_>>()
        );
        for _ in 0..2500 {
            survey.undo().unwrap();
        }
        assert_eq!(survey.survivors(), paths);
    }
}
