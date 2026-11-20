use std::collections::{HashMap, HashSet};

use finance_core::Category;

pub(super) fn category_tree_rows(categories: &[Category]) -> Vec<(usize, usize)> {
    let mut category_ids = HashSet::with_capacity(categories.len());
    let mut children_by_parent = HashMap::<i64, Vec<usize>>::new();
    let mut roots = Vec::new();

    for category in categories {
        category_ids.insert(category.id);
    }
    for (index, category) in categories.iter().enumerate() {
        match category.parent_id {
            Some(parent_id) if category_ids.contains(&parent_id) => {
                children_by_parent.entry(parent_id).or_default().push(index);
            }
            _ => roots.push(index),
        }
    }

    let mut rows = Vec::with_capacity(categories.len());
    let mut visited = vec![false; categories.len()];
    for root in roots.into_iter().chain(0..categories.len()) {
        if visited[root] {
            continue;
        }

        let mut pending = vec![(root, 0)];
        while let Some((index, depth)) = pending.pop() {
            if visited[index] {
                continue;
            }

            visited[index] = true;
            rows.push((index, depth));
            if let Some(children) = children_by_parent.get(&categories[index].id) {
                pending.extend(children.iter().rev().map(|&child| (child, depth + 1)));
            }
        }
    }

    rows
}

pub(super) fn category_tree_labels(categories: &[Category]) -> Vec<(i64, String)> {
    let mut path = Vec::new();

    category_tree_rows(categories)
        .into_iter()
        .map(|(index, depth)| {
            path.truncate(depth);
            path.push(categories[index].name.as_str());
            (categories[index].id, path.join(" → "))
        })
        .collect()
}
