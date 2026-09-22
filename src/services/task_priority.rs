//! Pure request-local layering of admitted pairwise Task Preferences.
use crate::api::planning::TaskPriorityBody;
use std::collections::{BTreeMap, BTreeSet};
use ubu_core::core::{Preference, PreferenceOrder, PreferenceSubjects};

#[derive(Debug)]
pub struct TaskPriorities {
    pub tasks: Vec<TaskPriorityBody>,
    pub cycles: Vec<Vec<String>>,
}

impl TaskPriorities {
    pub fn order_keys(&self) -> BTreeMap<String, u32> {
        self.tasks
            .iter()
            .map(|task| {
                let m = task.bucket_count;
                (
                    task.task_id.clone(),
                    task.bucket.unwrap_or(if m >= 2 { m - 1 } else { m }),
                )
            })
            .collect()
    }
}

fn root(parents: &[usize], mut node: usize) -> usize {
    while parents[node] != node {
        node = parents[node];
    }
    node
}

// Iterative postorder avoids recursion depth depending on the Task backlog.
fn finish_order(graph: &[BTreeSet<usize>]) -> Vec<usize> {
    let mut seen = vec![false; graph.len()];
    let mut order = Vec::new();
    for start in 0..graph.len() {
        let mut stack = vec![(start, false)];
        while let Some((node, finished)) = stack.pop() {
            if finished {
                order.push(node);
                continue;
            }
            if seen[node] {
                continue;
            }
            seen[node] = true;
            stack.push((node, true));
            stack.extend(graph[node].iter().rev().map(|&next| (next, false)));
        }
    }
    order
}

pub fn layer_preferences(eligible: &[String], preferences: &[Preference]) -> TaskPriorities {
    let ids: Vec<_> = eligible
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let index: BTreeMap<_, _> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let pairs: Vec<_> = preferences
        .iter()
        .filter(|p| p.enabled)
        .filter_map(|p| {
            let PreferenceSubjects::Tasks { a, b } = &p.subjects else {
                return None;
            };
            Some((*index.get(a.as_str())?, *index.get(b.as_str())?, p.order))
        })
        .collect();
    let mut parents: Vec<_> = (0..ids.len()).collect();
    let mut ranked = vec![false; ids.len()];
    for &(a, b, relation) in &pairs {
        ranked[a] = true;
        ranked[b] = true;
        if relation == PreferenceOrder::AIndifferentToB {
            let a = root(&parents, a);
            let b = root(&parents, b);
            parents[a.max(b)] = a.min(b);
        }
    }
    let roots: Vec<_> = (0..ids.len())
        .filter(|&i| ranked[i] && root(&parents, i) == i)
        .collect();
    let nodes: BTreeMap<_, _> = roots.iter().enumerate().map(|(i, &r)| (r, i)).collect();
    let mut graph = vec![BTreeSet::new(); roots.len()];
    let mut reverse = graph.clone();
    let mut contradictory = vec![false; roots.len()];
    for &(a, b, relation) in &pairs {
        if relation != PreferenceOrder::APreferredToB {
            continue;
        }
        let a = nodes[&root(&parents, a)];
        let b = nodes[&root(&parents, b)];
        if a == b {
            contradictory[a] = true;
        } else {
            graph[a].insert(b);
            reverse[b].insert(a);
        }
    }
    let mut component = vec![usize::MAX; graph.len()];
    let mut components: Vec<Vec<usize>> = Vec::new();
    for start in finish_order(&graph).into_iter().rev() {
        if component[start] != usize::MAX {
            continue;
        }
        let c = components.len();
        let mut members = Vec::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            if component[node] != usize::MAX {
                continue;
            }
            component[node] = c;
            members.push(node);
            stack.extend(reverse[node].iter().copied());
        }
        components.push(members);
    }
    let mut cycles = Vec::new();
    for (c, members) in components.iter().enumerate() {
        if members.len() > 1 || members.iter().any(|&node| contradictory[node]) {
            cycles.push(
                ids.iter()
                    .enumerate()
                    .filter(|(i, _)| ranked[*i] && component[nodes[&root(&parents, *i)]] == c)
                    .map(|(_, id)| id.clone())
                    .collect::<Vec<_>>(),
            );
        }
    }
    cycles.sort();
    let mut dag = vec![BTreeSet::new(); components.len()];
    let mut indegree = vec![0; components.len()];
    for (a, edges) in graph.iter().enumerate() {
        for &b in edges {
            let (a, b) = (component[a], component[b]);
            if a != b && dag[a].insert(b) {
                indegree[b] += 1;
            }
        }
    }
    let mut layer = vec![0u32; components.len()];
    let mut ready: BTreeSet<_> = (0..components.len())
        .filter(|&i| indegree[i] == 0)
        .collect();
    while let Some(a) = ready.pop_first() {
        for &b in &dag[a] {
            layer[b] = layer[b].max(layer[a] + 1);
            indegree[b] -= 1;
            if indegree[b] == 0 {
                ready.insert(b);
            }
        }
    }
    let m = layer.iter().max().map_or(0, |p| p + 1);
    let tasks = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let bucket = ranked[i].then(|| layer[component[nodes[&root(&parents, i)]]]);
            let normalized_rank = bucket.map(|p| {
                if m == 1 {
                    0.0
                } else {
                    p as f64 / (m - 1) as f64
                }
            });
            let value = match bucket {
                None => 0.1,
                Some(0) => 1.0,
                Some(p) if p == m - 1 => 0.1,
                Some(p) => 1.0 - 0.9 * p as f64 / (m - 1) as f64,
            };
            TaskPriorityBody {
                task_id: id.clone(),
                bucket,
                bucket_count: m,
                normalized_rank,
                value,
            }
        })
        .collect();
    TaskPriorities { tasks, cycles }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(c: char) -> String {
        format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e{:02x}", c as u8)
    }
    fn preference(a: char, b: char, indifferent: bool) -> Preference {
        serde_json::from_value(json!({
            "id":"pref_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f", "task_a":id(a), "task_b":id(b),
            "order":if indifferent {"a_indifferent_to_b"} else {"a_preferred_to_b"},
            "acquired_method":"user_defined", "acquired_date":"2026-09-21T12:00:00Z", "enabled":true,
            "provenance":{"created_at":"2026-09-21T12:00:00Z", "authority_source":"user"}
        })).unwrap()
    }
    fn layers(tasks: &str, edges: &[(char, char, bool)]) -> TaskPriorities {
        layer_preferences(
            &tasks.chars().map(id).collect::<Vec<_>>(),
            &edges
                .iter()
                .map(|&(a, b, tie)| preference(a, b, tie))
                .collect::<Vec<_>>(),
        )
    }
    fn task(result: &TaskPriorities, c: char) -> &TaskPriorityBody {
        result.tasks.iter().find(|t| t.task_id == id(c)).unwrap()
    }
    fn values(result: &TaskPriorities, expected: &[(char, f64)]) {
        for &(c, v) in expected {
            assert!((task(result, c).value - v).abs() < 1e-12, "{result:?}");
        }
    }
    #[test]
    fn chain_and_unranked_values() {
        let r = layers("ABCD", &[('A', 'B', false), ('B', 'C', false)]);
        values(&r, &[('A', 1.0), ('B', 0.55), ('C', 0.1), ('D', 0.1)]);
        assert_eq!(task(&r, 'D').bucket, None);
        assert_eq!(task(&r, 'D').normalized_rank, None);
        assert_eq!(task(&r, 'B').normalized_rank, Some(0.5));
        assert_eq!(r.order_keys()[&id('C')], r.order_keys()[&id('D')]);
    }
    #[test]
    fn indifference_shares_a_bucket() {
        let r = layers("ABC", &[('A', 'B', true), ('A', 'C', false)]);
        assert_eq!(task(&r, 'A').bucket, Some(0));
        assert_eq!(task(&r, 'B').bucket, Some(0));
        assert_eq!(task(&r, 'C').bucket, Some(1));
        values(&r, &[('A', 1.0), ('B', 1.0), ('C', 0.1)]);
    }
    #[test]
    fn buckets_use_longest_not_shortest_path() {
        let r = layers(
            "ABCD",
            &[('A', 'C', false), ('B', 'D', false), ('D', 'C', false)],
        );
        values(&r, &[('A', 1.0), ('B', 1.0), ('D', 0.55), ('C', 0.1)]);
        assert_eq!(task(&r, 'C').bucket, Some(2));
    }
    #[test]
    fn cycles_merge_and_accept_a_predecessor() {
        let edges = [('A', 'B', false), ('B', 'C', false), ('C', 'A', false)];
        let r = layers("ABC", &edges);
        values(&r, &[('A', 1.0), ('B', 1.0), ('C', 1.0)]);
        assert_eq!(r.cycles, vec![vec![id('A'), id('B'), id('C')]]);
        assert_eq!(task(&r, 'A').bucket_count, 1);
        let mut edges = edges.to_vec();
        edges.push(('D', 'A', false));
        let r = layers("ABCD", &edges);
        values(&r, &[('D', 1.0), ('A', 0.1), ('B', 0.1), ('C', 0.1)]);
        assert_eq!(r.cycles.len(), 1);
    }
    #[test]
    fn strict_edge_inside_indifference_is_a_cycle() {
        let r = layers("AB", &[('A', 'B', false), ('A', 'B', true)]);
        assert_eq!(r.cycles, vec![vec![id('A'), id('B')]]);
        values(&r, &[('A', 1.0), ('B', 1.0)]);
    }
    #[test]
    fn ignores_ineligible_disabled_and_objective_preferences() {
        let mut disabled = preference('A', 'B', false);
        disabled.enabled = false;
        let mut objective = preference('A', 'B', false);
        objective.subjects = PreferenceSubjects::Objectives {
            a: ubu_core::UbuId::parse(id('A').replace("task_", "obj_")).unwrap(),
            b: ubu_core::UbuId::parse(id('B').replace("task_", "obj_")).unwrap(),
        };
        let r = layer_preferences(
            &[id('A'), id('B')],
            &[preference('A', 'C', false), disabled, objective],
        );
        values(&r, &[('A', 0.1), ('B', 0.1)]);
        assert!(r
            .tasks
            .iter()
            .all(|t| t.bucket_count == 0 && t.bucket.is_none()));
    }
    #[test]
    fn one_indifference_bucket_does_not_rank_unrelated_tasks() {
        let r = layers("ABC", &[('A', 'B', true)]);
        values(&r, &[('A', 1.0), ('B', 1.0), ('C', 0.1)]);
        assert_eq!(task(&r, 'A').normalized_rank, Some(0.0));
        assert_eq!(r.order_keys()[&id('C')], 1);
    }
    #[test]
    fn lowest_endpoint_is_exact_for_two_and_ten_layers() {
        for n in [2, 10] {
            let chars: Vec<_> = (0..n).map(|i| (b'A' + i) as char).collect();
            let edges: Vec<_> = chars.windows(2).map(|w| (w[0], w[1], false)).collect();
            let r = layers(&chars.iter().collect::<String>(), &edges);
            assert_eq!(task(&r, *chars.last().unwrap()).value, 0.1);
            assert_eq!(task(&r, 'A').bucket_count, u32::from(n));
        }
    }
}
