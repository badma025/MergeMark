#!/usr/bin/env python3
"""Python mirror of validate.rs semantic figure validation and false-positive detection."""

import re

# Figure kinds that indicate semantic figures
SEMANTIC_FIGURE_KINDS = [
    "graph", "schema", "flowchart", "circuit", "diagram", "network",
    "tree", "chart", "plot", "logic gate", "state diagram",
    "entity relationship", "er diagram", "class diagram", "sequence diagram",
    "activity diagram", "use case", "gantt", "timeline", "multi-panel",
]

VALID_FIGURE_KINDS = [
    "graph", "schema", "flowchart", "circuit", "multi-panel",
    "diagram", "chart", "plot", "network", "tree", "timeline",
    "gantt", "state diagram", "entity relationship", "class diagram",
    "sequence diagram", "activity diagram", "use case",
]

ANSWER_GRID_PHRASES = [
    "complete the trace table", "complete the table", "complete the grid",
    "show the results of executing", "show your working", "contents of memory location"
]

CODE_KEYWORDS = [
    "function", "procedure", "if ", "else", "while ", "for ", "return ",
    "var ", "let ", "const ", "int ", "float ", "bool ", "string ",
    "print", "input", "output", "begin", "end", "then", "do ",
    "public ", "private ", "class ", "def ", "import ", "from ",
    "select ", "from ", "where ", "insert ", "update ", "delete ",
]

FOOTER_PATTERNS = [
    "page ", "paper ", "total for question", "marks",
    "copyright", "©", "aqa", "edexcel", "ocr", "wjec",
    "specimen", "version", "draft", "confidential",
]

MARGIN_FRAC = 0.05


def figure_references(content):
    """Count Figure N references."""
    return len(re.findall(r'(?i)\bfig(?:ure)?\.?\s*\d+', content))


def figure_reference_numbers(content):
    """Extract figure numbers from references."""
    return [int(m) for m in re.findall(r'(?i)\bfig(?:ure)?\.?\s*(\d+)', content)]


def is_answer_grid_request(content):
    """Check if content indicates an answer grid/trace table."""
    s = content.lower()
    return any(phrase in s for phrase in ANSWER_GRID_PHRASES)


def looks_like_semantic_figure(content):
    """Check if content suggests a legitimate figure type."""
    s = content.lower()
    return any(kind in s for kind in SEMANTIC_FIGURE_KINDS)


def estimate_text_density(content):
    """Estimate text density 0.0-1.0."""
    if not content.strip():
        return 0.0
    lines = content.split('\n')
    if not lines:
        return 0.0
    non_ws = sum(1 for c in content if not c.isspace())
    total = max(len(content), 1)
    density = non_ws / total
    avg_line_len = sum(len(l) for l in lines) / len(lines)
    line_factor = min(avg_line_len / 80.0, 1.0)
    return min(density * 0.7 + line_factor * 0.3, 1.0)


def looks_like_code_block(content):
    """Detect code-block-like content."""
    s = content.lower()
    lines = content.split('\n')
    if len(lines) < 3:
        return False
    keyword_hits = sum(1 for kw in CODE_KEYWORDS if kw in s)
    indented = sum(1 for l in lines if l.startswith('    ') or l.startswith('\t'))
    indent_ratio = indented / len(lines)
    return keyword_hits >= 2 or indent_ratio > 0.3


def looks_like_markdown_table(content):
    """Detect markdown-eligible table."""
    lines = content.split('\n')
    if len(lines) < 3:
        return False
    has_pipes = sum(1 for l in lines if '|' in l)
    has_sep = any('---' in l and '|' in l for l in lines)
    return has_pipes >= 2 and has_sep


def looks_like_footer(content):
    """Detect footer-like content."""
    if len(content) >= 200:
        return False
    s = content.lower()
    return any(p in s for p in FOOTER_PATTERNS)


def false_positive_crop_signals(content, bbox, page_width, page_height,
                                 has_caption_ref, has_visual_structure):
    """Return list of rejection reasons for false positive crops."""
    signals = []
    s = content.lower()
    
    x, y, w, h = bbox[0], bbox[1], bbox[2], bbox[3]
    
    # Position near margins
    if y < MARGIN_FRAC:
        signals.append("crop touches top margin")
    if y + h > 1.0 - MARGIN_FRAC:
        signals.append("crop touches bottom margin (likely footer)")
    if x < MARGIN_FRAC or x + w > 1.0 - MARGIN_FRAC:
        signals.append("crop touches side margin")
    
    # High text density without visual structure
    text_density = estimate_text_density(content)
    if text_density > 0.8 and not has_visual_structure and not has_caption_ref:
        signals.append("high text density without visual structure or caption")
    
    # Code block
    if looks_like_code_block(content) and not has_caption_ref:
        signals.append("code block without figure caption/reference")
    
    # Markdown table
    if looks_like_markdown_table(content) and not has_caption_ref:
        signals.append("markdown-eligible table without figure caption")
    
    # Footer
    if looks_like_footer(content):
        signals.append("footer/page identifier content")
    
    # Turn over
    if "turn over" in s or "continued" in s:
        signals.append('"turn over" or continuation area')
    
    # Barcode/QR
    if w < 0.15 and h < 0.15 and (x < 0.1 or x > 0.9 or y < 0.1 or y > 0.9):
        signals.append("small corner region (possible barcode/QR)")
    
    # Answer grid
    if is_answer_grid_request(content):
        signals.append("student answer grid / trace table instruction")
    
    # No evidence
    if not has_caption_ref and not has_visual_structure and not looks_like_semantic_figure(content):
        signals.append("no caption/reference and no visual structure evidence")
    
    return signals


def validate_figure_metadata(proposed_captions, proposed_kinds, page_text,
                              figure_refs, bbox_page_idx, total_pages):
    """Validate semantic figure metadata against page text."""
    errors = []
    page_text_lower = page_text.lower()
    
    for i, (caption, kind) in enumerate(zip(proposed_captions, proposed_kinds)):
        caption_lower = caption.lower()
        kind_lower = kind.lower()
        
        # Caption should appear in page text
        caption_words = caption_lower.split()
        meaningful = [w for w in caption_words 
                      if len(w) > 3 and w not in ["figure", "fig", "the", "and", "shows", "showing"]]
        if meaningful and not any(w in page_text_lower for w in meaningful):
            errors.append(f"figure {i+1}: caption '{caption}' not found in page text")
        
        # Kind should be recognized
        if kind_lower and not any(k in kind_lower for k in VALID_FIGURE_KINDS):
            errors.append(f"figure {i+1}: unrecognized kind '{kind}'")
        
        # Referenced figures should correspond
        for ref_num in figure_refs:
            ref_str = f"figure {ref_num}"
            if ref_str in caption_lower or ref_str in page_text_lower:
                pass  # Good - reference exists
    
    # Count mismatch
    ref_count = len(figure_refs)
    proposed_count = max(len(proposed_captions), len(proposed_kinds))
    if ref_count > 0 and proposed_count == 0:
        errors.append(f"content references {ref_count} figure(s) but no figure metadata proposed")
    
    return errors


# Tests
def test_figure_references():
    content = "Figure 1 shows the graph. Figure 2 is a table."
    assert figure_references(content) == 2
    assert figure_reference_numbers(content) == [1, 2]
    print("  ok: figure_references")


def test_is_answer_grid_request():
    assert is_answer_grid_request("Complete the trace table below")
    assert is_answer_grid_request("Complete the table for question 3")
    assert is_answer_grid_request("Show the results of executing the code")
    assert not is_answer_grid_request("This is a normal paragraph")
    print("  ok: is_answer_grid_request")


def test_looks_like_semantic_figure():
    assert looks_like_semantic_figure("Figure 1: A graph of the function")
    assert looks_like_semantic_figure("The flowchart shows the process")
    assert looks_like_semantic_figure("Circuit diagram for the logic gate")
    assert not looks_like_semantic_figure("This is just plain text")
    print("  ok: looks_like_semantic_figure")


def test_false_positive_prose():
    content = "This is a long paragraph of text with many words that continues for several lines without any figure reference or caption."
    bbox = [0.1, 0.1, 0.8, 0.3]
    signals = false_positive_crop_signals(content, bbox, 1000, 1000, False, False)
    assert "high text density without visual structure or caption" in signals
    print("  ok: false_positive_prose")


def test_false_positive_code():
    content = "function main() {\n    var x = 1;\n    if (x > 0) {\n        return x;\n    }\n}"
    bbox = [0.1, 0.1, 0.5, 0.5]
    signals = false_positive_crop_signals(content, bbox, 1000, 1000, False, False)
    assert "code block without figure caption/reference" in signals
    print("  ok: false_positive_code")


def test_false_positive_table():
    content = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |"
    bbox = [0.1, 0.1, 0.5, 0.3]
    signals = false_positive_crop_signals(content, bbox, 1000, 1000, False, False)
    assert "markdown-eligible table without figure caption" in signals
    print("  ok: false_positive_table")


def test_false_positive_footer():
    content = "Page 5 of 10  AQA  Copyright 2024"
    bbox = [0.1, 0.95, 0.8, 0.05]
    signals = false_positive_crop_signals(content, bbox, 1000, 1000, False, False)
    assert "footer/page identifier content" in signals
    print("  ok: false_positive_footer")


def test_false_positive_answer_grid():
    content = "Complete the trace table below."
    bbox = [0.1, 0.1, 0.8, 0.8]
    signals = false_positive_crop_signals(content, bbox, 1000, 1000, False, False)
    assert "student answer grid / trace table instruction" in signals
    print("  ok: false_positive_answer_grid")


def test_validate_figure_metadata():
    captions = ["Figure 1: Graph of y=x^2"]
    kinds = ["graph"]
    page_text = "Figure 1 shows the graph of the function."
    refs = [1]
    errors = validate_figure_metadata(captions, kinds, page_text, refs, 0, 5)
    assert len(errors) == 0
    
    # Bad caption
    captions = ["Unknown figure"]
    errors = validate_figure_metadata(captions, kinds, page_text, refs, 0, 5)
    assert any("not found in page text" in e for e in errors)
    print("  ok: validate_figure_metadata")


def test_estimate_text_density():
    # Prose
    prose = "This is a sentence. This is another sentence. And more text here."
    assert estimate_text_density(prose) > 0.5
    
    # Sparse
    sparse = "a\nb\nc"
    assert estimate_text_density(sparse) < 0.5
    print("  ok: estimate_text_density")


def test_looks_like_code_block():
    code = "def func():\n    x = 1\n    return x\n"
    assert looks_like_code_block(code)
    
    not_code = "This is normal text."
    assert not looks_like_code_block(not_code)
    print("  ok: looks_like_code_block")


if __name__ == "__main__":
    test_figure_references()
    test_is_answer_grid_request()
    test_looks_like_semantic_figure()
    test_false_positive_prose()
    test_false_positive_code()
    test_false_positive_table()
    test_false_positive_footer()
    test_false_positive_answer_grid()
    test_validate_figure_metadata()
    test_estimate_text_density()
    test_looks_like_code_block()
    print("\nALL VALIDATE MIRROR TESTS PASSED")
