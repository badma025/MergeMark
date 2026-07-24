fn main() {
    let re = regex::Regex::new(r"(?m)(?:^|\n)\s*(?:\*\*)?(?:Q(?:uestion)?\.?\s*)?([0-9](?:\s*[0-9]){0,2})(?:\*\*)?\s*(?:[\.\)\]\---]|\s+)(?:\D|$)").unwrap();
    let texts = vec![
        "0 1 A capacitor",
        "0 1 . 1 Explain",
        "1 0 Three particles",
        "3 1 27Mg",
        "1. ",
        "Q2. ",
        "2021 ",
    ];
    for t in texts {
        if let Some(cap) = re.captures(t) {
            let num_str = cap[1].replace(" ", "");
            let num = num_str.parse::<u32>().unwrap();
            println!("{:?} -> {}", t, num);
        } else {
            println!("{:?} -> NO MATCH", t);
        }
    }
}
