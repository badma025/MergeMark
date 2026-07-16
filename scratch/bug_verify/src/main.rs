// Verification of suspected determinism bugs in src-tauri/src/commands.rs
// 1) fix_json_escapes corrupting single-backslash LaTeX (\nabla, \tan, \to)
// 2) crop coordinate math when AI returns pixel-integer bboxes instead of 0.0-1.0 relative

fn fix_json_escapes(s: &str) -> String {
    // — exact copy of commands.rs:658 —
    let mut out = String::with_capacity(s.len() + 100);
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            if i + 1 < chars.len() {
                let next = chars[i + 1];
                if next == '"' || next == '\\' || next == '/' || next == 'b' || next == 'f' || next == 'n' || next == 'r' || next == 't' || next == 'u' {
                    out.push('\\');
                    out.push(next);
                    i += 2;
                    continue;
                } else {
                    out.push('\\');
                    out.push('\\');
                    out.push(next);
                    i += 2;
                    continue;
                }
            } else {
                out.push('\\');
                out.push('\\');
                i += 1;
                continue;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn main() {
    println!("=== BUG 1: fix_json_escapes LaTeX corruption ===");
    // Simulates AI emitting VALID JSON string escapes for real newlines plus
    // single-backslash LaTeX (the exact thing the fixer exists to repair).
    let ai_output = r#"{"content":"Evaluate \nabla f and \tan \theta, then \n\to value"}"#;
    let fixed = fix_json_escapes(ai_output);
    match serde_json::from_str::<serde_json::Value>(&fixed) {
        Ok(v) => {
            let content = v["content"].as_str().unwrap().to_string();
            println!("parsed content: {:?}\n", content);
            for bad in ["\\nabla", "\\tan", "\\to"] {
                println!("  contains {}? {}", bad, content.contains(bad));
            }
            println!("  contains raw newline? {}", content.contains('\n'));
            println!("  contains raw tab? {}", content.contains('\t'));
        }
        Err(e) => println!("PARSE FAILED: {e}\nfixed string: {fixed}"),
    }

    println!("\n=== BUG 2: crop math with pixel-integer bbox (AI misbehaviour) ===");
    // Simulates 1654x2339px rendered page (A4 @ scale 2), AI returns pixels.
    let width: u32 = 1654;
    let height: u32 = 2339;
    let bbox = [100.0_f32, 150.0, 600.0, 400.0]; // pixel ints (misbehaviour); relative would be [0.06, 0.06, 0.36, 0.17]
    let min_x = bbox[0]; let min_y = bbox[1];
    let merged_w = bbox[2]; let merged_h = bbox[3];
    let x = (min_x * width as f32) as u32;
    let y = (min_y * height as f32) as u32;
    let w = (merged_w * width as f32) as u32;
    let h = (merged_h * height as f32) as u32;
    let padding: u32 = 40;
    let safe_x = x.saturating_sub(padding);
    let safe_y = y.saturating_sub(padding);
    println!("x={x} y={y} w={w} h={h} safe_x={safe_x} safe_y={safe_y} img={width}x{height}");
    // This is the exact next line from commands.rs:1232 (debug build):
    let safe_width = (w + (x - safe_x) + padding).min(width - safe_x); // width - safe_x underflows if safe_x > width
    let safe_height = (h + (y - safe_y) + padding).min(height - safe_y);
    println!("safe_width={safe_width} safe_height={safe_height} (would then call imageops::crop)");
}
