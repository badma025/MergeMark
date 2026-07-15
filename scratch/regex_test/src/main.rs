use regex::Regex;

fn main() {
    // Regex pattern for:
    // "1." or "1" (sometimes at the start of a line)
    // "1(a)" or "1 (a)"
    // "Question 1"
    
    // Pattern breakdown:
    // ^\s* matches optional leading space
    // (?:
    //   Question\s+(\d+)    # "Question 1"
    //   |
    //   (\d+)\.?\s*(?:\([a-z]\))?  # "1", "1.", "1(a)", "1 (a)"
    // )
    // (?:\s+|$)  # Followed by space or end of line to prevent matching "100" as "10"

    let pattern = r#"^\s*(?:Question\s+(?P<q1>\d+)|(?P<q2>\d+)\.?\s*(?:\([a-z]\))?)(?:\s+|$)"#;
    let re = Regex::new(pattern).unwrap();

    let tests = vec![
        "1. Find the value of",
        "1 Find the value of",
        "1(a) Find",
        "1 (a) Find",
        "Question 1",
        "Question 12",
        "12(b) Solve",
        " 2. Solve",
        "100 Find",   // Should match q=100
        "1A Find",    // Should not match as question start unless we allow uppercase? The prompt said 1(a), so we'll stick to \([a-z]\).
        "Hello 1",    // Should not match
    ];

    for t in tests {
        if let Some(caps) = re.captures(t) {
            let q_num_str = caps.name("q1").or_else(|| caps.name("q2")).unwrap().as_str();
            println!("Matched '{}' -> Question {}", t, q_num_str);
        } else {
            println!("Did not match '{}'", t);
        }
    }
}
