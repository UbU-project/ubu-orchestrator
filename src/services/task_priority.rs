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
