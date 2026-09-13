const FORSYTH_CACHE_SIZE: i32 = 32;

#[inline]
fn forsyth_vertex_score(cache_pos: i32, active_tris: u32) -> f32 {
    if active_tris == 0 {
        return -1.0;
    }
    let mut score = 0.0f32;
    if cache_pos >= 0 {
        if cache_pos < 3 {
            score = 0.75;
        } else {
            let scaler = 1.0 - (cache_pos - 3) as f32 / (FORSYTH_CACHE_SIZE - 3) as f32;
            score = scaler.powf(1.5);
        }
    }
    score + 2.0 * (active_tris as f32).powf(-0.5)
}

/// Safe wrapper: reorder triangle indices for vertex cache locality.
/// Same routine the terrain kernel uses, shared here for airframe parts.
pub fn reorder(input: &[u32]) -> Vec<u32> {
    assert!(input.len() % 3 == 0);
    let mut out = vec![0u32; input.len()];
    optimize_indices(input.as_ptr(), input.len() / 3, out.as_mut_ptr());
    out
}

#[no_mangle]
pub extern "C" fn optimize_indices(indices: *const u32, tri_count: usize, out: *mut u32) {
    if tri_count == 0 {
        return;
    }
    let input: Vec<u32> = unsafe { std::slice::from_raw_parts(indices, tri_count * 3).to_vec() };
    let max_vertex = input.iter().copied().max().unwrap_or(0) as usize;
    if max_vertex > 4_000_000 {
        unsafe {
            std::ptr::copy_nonoverlapping(input.as_ptr(), out, tri_count * 3);
        }
        return;
    }
    let nverts = max_vertex + 1;

    let mut counts = vec![0u32; nverts];
    for &v in &input {
        counts[v as usize] += 1;
    }
    let mut offsets = vec![0usize; nverts + 1];
    for i in 0..nverts {
        offsets[i + 1] = offsets[i] + counts[i] as usize;
    }
    let mut adjacency = vec![0u32; tri_count * 3];
    let mut cursor = offsets[..nverts].to_vec();
    for (t, tri) in input.chunks_exact(3).enumerate() {
        for &v in tri {
            let slot = cursor[v as usize];
            adjacency[slot] = t as u32;
            cursor[v as usize] = slot + 1;
        }
    }
    let mut active = counts;
    let mut cache_pos = vec![-1i32; nverts];
    let mut cache: Vec<u32> = Vec::with_capacity(FORSYTH_CACHE_SIZE as usize);
    let mut tri_score = vec![0.0f32; tri_count];
    for (t, tri) in input.chunks_exact(3).enumerate() {
        let mut s = 0.0f32;
        for &v in tri {
            s += forsyth_vertex_score(-1, active[v as usize]);
        }
        tri_score[t] = s;
    }
    let mut added = vec![false; tri_count];
    let mut remaining = tri_count;
    let mut out_tri = 0usize;

    let mut best: Option<usize> = (0..tri_count).max_by(|&a, &b| {
        tri_score[a]
            .partial_cmp(&tri_score[b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    unsafe {
        while remaining > 0 {
            let t = match best {
                Some(t) if !added[t] => t,
                _ => match (0..tri_count).filter(|&t| !added[t]).max_by(|&a, &b| {
                    tri_score[a]
                        .partial_cmp(&tri_score[b])
                        .unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    Some(t) => t,
                    None => break,
                },
            };
            added[t] = true;
            remaining -= 1;
            let base = t * 3;
            *out.add(out_tri * 3) = input[base];
            *out.add(out_tri * 3 + 1) = input[base + 1];
            *out.add(out_tri * 3 + 2) = input[base + 2];
            out_tri += 1;
            best = None;
            for k in 0..3 {
                let v = input[base + k] as usize;

                if let Some(i) = cache.iter().position(|&u| u as usize == v) {
                    for &u in cache.iter().take(i) {
                        cache_pos[u as usize] += 1;
                    }
                    cache.remove(i);
                } else {
                    for &u in cache.iter() {
                        cache_pos[u as usize] += 1;
                    }
                }
                cache.insert(0, v as u32);
                if cache.len() > FORSYTH_CACHE_SIZE as usize {
                    if let Some(evicted) = cache.pop() {
                        cache_pos[evicted as usize] = -1;
                    }
                }
                cache_pos[v] = 0;
                if active[v] > 0 {
                    active[v] -= 1;
                }
                for s in offsets[v]..offsets[v + 1] {
                    let n = adjacency[s] as usize;
                    if added[n] {
                        continue;
                    }
                    let nb = n * 3;
                    tri_score[n] = forsyth_vertex_score(
                        cache_pos[input[nb] as usize],
                        active[input[nb] as usize],
                    ) + forsyth_vertex_score(
                        cache_pos[input[nb + 1] as usize],
                        active[input[nb + 1] as usize],
                    ) + forsyth_vertex_score(
                        cache_pos[input[nb + 2] as usize],
                        active[input[nb + 2] as usize],
                    );
                    match best {
                        Some(b) if tri_score[n] <= tri_score[b] => {}
                        _ => best = Some(n),
                    }
                }
            }
        }
    }
}
