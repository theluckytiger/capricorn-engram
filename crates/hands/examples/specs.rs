//! How big her tools are as a model sees them: each tool's description and parameters, in characters.

fn main() {
    let mut all: Vec<(String, usize)> = capri_hands::all().specs().into_iter().map(|t| (t.name.clone(), serde_json::to_string(&t).unwrap().len())).collect();
    all.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let total: usize = all.iter().map(|(_, n)| n).sum();
    for (name, n) in &all {
        println!("{n:>6}  {name}");
    }
    println!("{total:>6}  in all ({} tools, ~{} tokens)", all.len(), total / 4);
}
