// ABOUTME: Deterministic Leiden (modularity objective) over a weighted undirected graph.
// ABOUTME: Greedy refinement instead of the paper's random choice; communities end up connected.

use std::collections::{BTreeMap, HashSet, VecDeque};

const EPS: f64 = 1e-12;

/// Undirected weighted graph without self-loops; a repeated pair is ignored.
#[derive(Debug, Clone, Default)]
pub struct WeightedGraph {
    adj: Vec<Vec<(usize, f64)>>,
    seen: HashSet<(usize, usize)>,
}

impl WeightedGraph {
    pub fn new(n: usize) -> Self {
        Self {
            adj: vec![Vec::new(); n],
            seen: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.adj.len()
    }

    pub fn is_empty(&self) -> bool {
        self.adj.is_empty()
    }

    pub fn add_edge(&mut self, a: usize, b: usize, w: f64) {
        if a == b || !self.seen.insert((a.min(b), a.max(b))) {
            return;
        }
        self.adj[a].push((b, w));
        self.adj[b].push((a, w));
    }

    pub fn neighbors(&self, i: usize) -> &[(usize, f64)] {
        &self.adj[i]
    }

    pub fn strength(&self, i: usize) -> f64 {
        self.adj[i].iter().map(|&(_, w)| w).sum()
    }
}

/// One level of the aggregation hierarchy; self-loops count twice in a node's strength.
struct Level {
    adj: Vec<Vec<(usize, f64)>>,
    loops: Vec<f64>,
    k: Vec<f64>,
    two_m: f64,
}

impl Level {
    fn from_graph(g: &WeightedGraph) -> Self {
        let mut adj = g.adj.clone();
        for row in &mut adj {
            row.sort_by_key(|&(j, _)| j);
        }
        Self::finish(adj, vec![0.0; g.len()])
    }

    fn finish(adj: Vec<Vec<(usize, f64)>>, loops: Vec<f64>) -> Self {
        let k: Vec<f64> = adj
            .iter()
            .zip(&loops)
            .map(|(row, l)| row.iter().map(|&(_, w)| w).sum::<f64>() + 2.0 * l)
            .collect();
        let two_m = k.iter().sum();
        Level {
            adj,
            loops,
            k,
            two_m,
        }
    }

    fn len(&self) -> usize {
        self.adj.len()
    }

    /// Collapses `groups` (ids 0..count) into single nodes.
    fn aggregate(&self, groups: &[usize], count: usize) -> Level {
        let mut loops = vec![0.0; count];
        let mut between: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        for a in 0..self.len() {
            loops[groups[a]] += self.loops[a];
            for &(b, w) in &self.adj[a] {
                if a < b {
                    let (ga, gb) = (groups[a], groups[b]);
                    if ga == gb {
                        loops[ga] += w;
                    } else {
                        *between.entry((ga.min(gb), ga.max(gb))).or_default() += w;
                    }
                }
            }
        }
        let mut adj = vec![Vec::new(); count];
        for ((a, b), w) in between {
            adj[a].push((b, w));
            adj[b].push((a, w));
        }
        for row in &mut adj {
            row.sort_by_key(|&(j, _)| j);
        }
        Level::finish(adj, loops)
    }
}

/// Partition maximising modularity with `resolution`; community ids follow the first node of each.
pub fn leiden(g: &WeightedGraph, resolution: f64, iterations: u32) -> Vec<usize> {
    let n = g.len();
    if n == 0 {
        return Vec::new();
    }
    let mut part: Vec<usize> = (0..n).collect();
    for _ in 0..iterations.max(1) {
        part = run_once(g, resolution, &part);
    }
    renumber(&split_components(g, &part))
}

fn run_once(g: &WeightedGraph, gamma: f64, init: &[usize]) -> Vec<usize> {
    let mut level = Level::from_graph(g);
    let mut comm = renumber(init);
    let mut assign: Vec<usize> = (0..g.len()).collect();
    loop {
        local_move(&level, &mut comm, gamma);
        let (refined, count) = refine(&level, &comm, gamma);
        if count == level.len() {
            break;
        }
        let mut next_comm = vec![0; count];
        for (node, &r) in refined.iter().enumerate() {
            next_comm[r] = comm[node];
        }
        level = level.aggregate(&refined, count);
        for a in &mut assign {
            *a = refined[*a];
        }
        comm = next_comm;
    }
    assign.iter().map(|&a| comm[a]).collect()
}

/// Queue-based node moving. Returns whether any node changed community.
fn local_move(level: &Level, comm: &mut [usize], gamma: f64) -> bool {
    let n = level.len();
    if level.two_m <= 0.0 {
        return false;
    }
    let mut sigma = vec![0.0; n.max(comm.iter().copied().max().map_or(0, |m| m + 1))];
    for i in 0..n {
        sigma[comm[i]] += level.k[i];
    }
    let mut queue: VecDeque<usize> = (0..n).collect();
    let mut queued = vec![true; n];
    let mut weight_to = vec![0.0; sigma.len()];
    let mut moved_any = false;
    while let Some(i) = queue.pop_front() {
        queued[i] = false;
        let current = comm[i];
        let mut touched: Vec<usize> = Vec::new();
        for &(j, w) in &level.adj[i] {
            let c = comm[j];
            if weight_to[c] == 0.0 {
                touched.push(c);
            }
            weight_to[c] += w;
        }
        sigma[current] -= level.k[i];
        let gain = |c: usize, w: f64| w - gamma * level.k[i] * sigma[c] / level.two_m;
        let mut best = current;
        let mut best_gain = gain(current, weight_to[current]);
        touched.sort_unstable();
        for &c in &touched {
            let g = gain(c, weight_to[c]);
            if g > best_gain + EPS {
                best = c;
                best_gain = g;
            }
        }
        sigma[best] += level.k[i];
        if best != current {
            comm[i] = best;
            moved_any = true;
            for &(j, _) in &level.adj[i] {
                if comm[j] != best && !queued[j] {
                    queued[j] = true;
                    queue.push_back(j);
                }
            }
        }
        for &c in &touched {
            weight_to[c] = 0.0;
        }
    }
    moved_any
}

/// Greedy refinement inside each community: singletons merge into the best neighbouring
/// sub-community of the same community. Returns sub-community ids (0..count) per node.
fn refine(level: &Level, comm: &[usize], gamma: f64) -> (Vec<usize>, usize) {
    let n = level.len();
    let mut sub: Vec<usize> = (0..n).collect();
    let mut size = vec![1usize; n];
    let mut sigma: Vec<f64> = level.k.clone();
    if level.two_m > 0.0 {
        let mut weight_to = vec![0.0; n];
        for i in 0..n {
            if size[sub[i]] != 1 || sub[i] != i {
                continue;
            }
            let mut touched: Vec<usize> = Vec::new();
            for &(j, w) in &level.adj[i] {
                if comm[j] != comm[i] {
                    continue;
                }
                let r = sub[j];
                if weight_to[r] == 0.0 {
                    touched.push(r);
                }
                weight_to[r] += w;
            }
            touched.sort_unstable();
            let mut best: Option<usize> = None;
            let mut best_gain = EPS;
            for &r in &touched {
                let g = weight_to[r] - gamma * level.k[i] * sigma[r] / level.two_m;
                if g > best_gain {
                    best = Some(r);
                    best_gain = g;
                }
            }
            for &r in &touched {
                weight_to[r] = 0.0;
            }
            if let Some(r) = best {
                sigma[i] -= level.k[i];
                size[i] -= 1;
                sub[i] = r;
                sigma[r] += level.k[i];
                size[r] += 1;
            }
        }
    }
    let renumbered = renumber(&sub);
    let count = renumbered.iter().copied().max().map_or(0, |m| m + 1);
    (renumbered, count)
}

/// Splits every community into its connected components over `g`.
fn split_components(g: &WeightedGraph, part: &[usize]) -> Vec<usize> {
    let n = g.len();
    let mut out = vec![usize::MAX; n];
    let mut next = 0;
    for start in 0..n {
        if out[start] != usize::MAX {
            continue;
        }
        out[start] = next;
        let mut stack = vec![start];
        while let Some(a) = stack.pop() {
            for &(b, _) in g.neighbors(a) {
                if out[b] == usize::MAX && part[b] == part[start] {
                    out[b] = next;
                    stack.push(b);
                }
            }
        }
        next += 1;
    }
    out
}

/// Renumbers ids 0..k by the order of each community's first node.
fn renumber(part: &[usize]) -> Vec<usize> {
    let mut map: BTreeMap<usize, usize> = BTreeMap::new();
    part.iter()
        .map(|&c| {
            let next = map.len();
            *map.entry(c).or_insert(next)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clique(g: &mut WeightedGraph, nodes: &[usize]) {
        for (i, &a) in nodes.iter().enumerate() {
            for &b in &nodes[i + 1..] {
                g.add_edge(a, b, 1.0);
            }
        }
    }

    fn count(part: &[usize]) -> usize {
        part.iter().copied().max().map_or(0, |m| m + 1)
    }

    fn modularity(g: &WeightedGraph, part: &[usize], gamma: f64) -> f64 {
        let k: Vec<f64> = (0..g.len()).map(|i| g.strength(i)).collect();
        let two_m: f64 = k.iter().sum();
        if two_m == 0.0 {
            return 0.0;
        }
        let mut q = 0.0;
        for a in 0..g.len() {
            for &(b, w) in g.neighbors(a) {
                if part[a] == part[b] {
                    q += w;
                }
            }
        }
        q /= two_m;
        let mut tot = vec![0.0; count(part)];
        for (i, &c) in part.iter().enumerate() {
            tot[c] += k[i];
        }
        q - gamma * tot.iter().map(|t| t * t).sum::<f64>() / (two_m * two_m)
    }

    #[test]
    fn two_cliques_with_bridge_split_in_two() {
        let mut g = WeightedGraph::new(10);
        clique(&mut g, &[0, 1, 2, 3, 4]);
        clique(&mut g, &[5, 6, 7, 8, 9]);
        g.add_edge(4, 5, 1.0);
        let part = leiden(&g, 1.0, 2);
        assert_eq!(count(&part), 2);
        assert!(part[..5].iter().all(|&c| c == part[0]));
        assert!(part[5..].iter().all(|&c| c == part[5]));
        assert_ne!(part[0], part[5]);
    }

    #[test]
    fn disconnected_components_never_share_a_community() {
        let mut g = WeightedGraph::new(6);
        g.add_edge(0, 1, 1.0);
        g.add_edge(2, 3, 1.0);
        g.add_edge(4, 5, 1.0);
        let part = leiden(&g, 1.0, 2);
        assert_eq!(count(&part), 3);
        assert_eq!(part[0], part[1]);
        assert_ne!(part[0], part[2]);
        assert_ne!(part[2], part[4]);
    }

    #[test]
    fn deterministic() {
        let g = lcg_graph(40, 110);
        assert_eq!(leiden(&g, 1.0, 2), leiden(&g, 1.0, 2));
    }

    #[test]
    fn higher_resolution_gives_more_communities() {
        let mut g = WeightedGraph::new(24);
        for ring in 0..6 {
            let base = ring * 4;
            clique(&mut g, &[base, base + 1, base + 2, base + 3]);
            g.add_edge(base + 3, (base + 4) % 24, 1.0);
        }
        let coarse = count(&leiden(&g, 0.5, 2));
        let fine = count(&leiden(&g, 2.0, 2));
        assert!(coarse <= fine, "coarse {coarse} fine {fine}");
        assert!(fine >= 6);
    }

    #[test]
    fn empty_and_edgeless() {
        assert!(leiden(&WeightedGraph::new(0), 1.0, 2).is_empty());
        assert_eq!(leiden(&WeightedGraph::new(3), 1.0, 2), vec![0, 1, 2]);
    }

    fn lcg_graph(n: usize, edges: usize) -> WeightedGraph {
        let mut g = WeightedGraph::new(n);
        let mut state: u64 = 12345;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..edges {
            let (a, b) = (next() % n, next() % n);
            if a != b {
                g.add_edge(a, b, 1.0);
            }
        }
        g
    }

    #[test]
    fn modularity_not_worse_than_singletons() {
        let g = lcg_graph(60, 150);
        let singletons: Vec<usize> = (0..60).collect();
        let part = leiden(&g, 1.0, 2);
        assert!(modularity(&g, &part, 1.0) >= modularity(&g, &singletons, 1.0));
        assert!(modularity(&g, &part, 1.0) > 0.0);
    }
}
