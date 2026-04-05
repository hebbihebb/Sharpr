use std::path::PathBuf;

use image::imageops::{self, FilterType};

pub fn dhash(img: &image::DynamicImage) -> u64 {
    let gray = img.to_luma8();
    let resized = imageops::resize(&gray, 9, 8, FilterType::Nearest);

    let mut hash = 0_u64;
    for row in 0..8 {
        for col in 0..8 {
            let left = resized.get_pixel(col, row)[0];
            let right = resized.get_pixel(col + 1, row)[0];
            hash <<= 1;
            if left > right {
                hash |= 1;
            }
        }
    }

    hash
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub fn group_duplicates(hashes: &[(PathBuf, u64)]) -> Vec<Vec<PathBuf>> {
    let len = hashes.len();
    if len < 2 {
        return Vec::new();
    }

    let mut adjacent = vec![vec![false; len]; len];
    let mut degree = vec![0_usize; len];
    for i in 0..len {
        adjacent[i][i] = true;
        for j in (i + 1)..len {
            let is_match = hamming(hashes[i].1, hashes[j].1) <= 4;
            adjacent[i][j] = is_match;
            adjacent[j][i] = is_match;
            if is_match {
                degree[i] += 1;
                degree[j] += 1;
            }
        }
    }

    let mut order: Vec<usize> = (0..len).collect();
    order.sort_by(|&a, &b| {
        degree[b]
            .cmp(&degree[a])
            .then_with(|| hashes[a].0.cmp(&hashes[b].0))
    });

    let mut assigned = vec![false; len];
    let mut groups = Vec::new();

    for seed in order {
        if assigned[seed] {
            continue;
        }

        let mut group = vec![seed];
        let mut candidates: Vec<usize> = (0..len)
            .filter(|&idx| !assigned[idx] && idx != seed && adjacent[seed][idx])
            .collect();
        candidates.sort_by(|&a, &b| {
            degree[b]
                .cmp(&degree[a])
                .then_with(|| hashes[a].0.cmp(&hashes[b].0))
        });

        for candidate in candidates {
            if group.iter().all(|&member| adjacent[member][candidate]) {
                group.push(candidate);
            }
        }

        if group.len() >= 2 {
            let mut paths: Vec<PathBuf> = group.iter().map(|&idx| hashes[idx].0.clone()).collect();
            paths.sort();
            for idx in group {
                assigned[idx] = true;
            }
            groups.push(paths);
        }
    }

    groups.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    groups
}

#[cfg(test)]
mod tests {
    use super::{group_duplicates, hamming};
    use std::path::PathBuf;

    #[test]
    fn hamming_distance_counts_bits() {
        assert_eq!(hamming(0b1010, 0b0011), 2);
    }

    #[test]
    fn groups_only_pairwise_close_entries() {
        let hashes = vec![
            (PathBuf::from("a.jpg"), 0),
            (PathBuf::from("b.jpg"), 0b1111),
            (PathBuf::from("c.jpg"), 0b11_1111),
            (PathBuf::from("d.jpg"), u64::MAX),
        ];

        let groups = group_duplicates(&hashes);
        assert_eq!(groups, vec![vec![PathBuf::from("a.jpg"), PathBuf::from("b.jpg")]]);
    }
}
