fn main() {
    let re = regex::Regex::new(
        r"(?m)(?:^|\n)\s*(?:\*\*)?(?:Q(?:uestion)?\.?\s*)?0*\s*([1-9](?:\s*\d)?)(?:\*\*)?\s*(?:[\.\)\]\-–—]|\s+)(?:\D|$)"
    ).unwrap();

    let text16 = "D halving the number of wires in the cable\n\n1 6 An electron enters a uniform magnetic field";
    let text13 = "D 3.0 V div−1\n\n1 3 A signal generator supplies a sinusoidal root mean square voltage";
    let text22 = "D 106\n m\n\n2 2 An asteroid has a mass of 2 × 1017 kg";

    for cap in re.captures_iter(text16) {
        println!("Match 16: {:?}", cap.get(1).unwrap().as_str());
    }
    for cap in re.captures_iter(text13) {
        println!("Match 13: {:?}", cap.get(1).unwrap().as_str());
    }
    for cap in re.captures_iter(text22) {
        println!("Match 22: {:?}", cap.get(1).unwrap().as_str());
    }
}
