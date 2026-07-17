#!/usr/bin/env python3
"""Python mirror of geometry.rs caption-aware crop expansion logic."""

MIN_EDGE_PX = 12
MAX_AREA_FRAC = 0.92
MIN_AREA_FRAC = 0.0005


class PixelRect:
    def __init__(self, x, y, w, h):
        self.x = x
        self.y = y
        self.w = w
        self.h = h


class CaptionHint:
    def __init__(self, x, y, above):
        self.x = x
        self.y = y
        self.above = above


def expand_bbox_for_caption(bbox, img_w, img_h, caption_hint, question_region=None):
    """Expand a sanitized bbox to include nearby caption text."""
    expanded = PixelRect(bbox.x, bbox.y, bbox.w, bbox.h)
    
    if caption_hint is None:
        return expanded
    
    caption_x = round(caption_hint.x * img_w)
    caption_y = round(caption_hint.y * img_h)
    
    MAX_EXPANSION_FRAC = 0.15
    MAX_EXPANSION_PX = 80
    
    vertical_expansion = min(round(bbox.h * MAX_EXPANSION_FRAC), MAX_EXPANSION_PX)
    horizontal_expansion = min(round(bbox.w * MAX_EXPANSION_FRAC), MAX_EXPANSION_PX)
    
    if caption_hint.above:
        new_y = max(expanded.y - vertical_expansion, 0)
        new_h = expanded.h + (expanded.y - new_y)
        if is_safe_expansion(new_y, new_h, expanded.x, expanded.w, img_w, img_h, question_region):
            expanded.y = new_y
            expanded.h = new_h
    else:
        new_h = min(expanded.h + vertical_expansion, img_h - expanded.y)
        if is_safe_expansion(expanded.y, new_h, expanded.x, expanded.w, img_w, img_h, question_region):
            expanded.h = new_h
    
    # Horizontal expansion for caption width
    if caption_x < expanded.x:
        expand_left = min(expanded.x - caption_x, horizontal_expansion)
        new_x = max(expanded.x - expand_left, 0)
        new_w = expanded.w + (expanded.x - new_x)
        if is_safe_expansion(expanded.y, expanded.h, new_x, new_w, img_w, img_h, question_region):
            expanded.x = new_x
            expanded.w = new_w
    elif caption_x > expanded.x + expanded.w:
        expand_right = min(caption_x - (expanded.x + expanded.w), horizontal_expansion)
        new_w = min(expanded.w + expand_right, img_w - expanded.x)
        if is_safe_expansion(expanded.y, expanded.h, expanded.x, new_w, img_w, img_h, question_region):
            expanded.w = new_w
    
    # Final clamp
    expanded.x = min(expanded.x, img_w - 1)
    expanded.y = min(expanded.y, img_h - 1)
    expanded.w = min(expanded.w, img_w - expanded.x)
    expanded.h = min(expanded.h, img_h - expanded.y)
    
    if expanded.w < MIN_EDGE_PX or expanded.h < MIN_EDGE_PX:
        return bbox  # Revert
    
    return expanded


def is_safe_expansion(y, h, x, w, img_w, img_h, question_region):
    # Page bounds
    if x + w > img_w or y + h > img_h:
        return False
    
    # Footer zone (bottom 8%)
    footer_start = round(img_h * 0.92)
    if y + h > footer_start and y < img_h:
        return False
    
    # Header zone (top 5%)
    header_end = round(img_h * 0.05)
    if y < header_end:
        return False
    
    # Side margins (5%)
    left_margin = round(img_w * 0.05)
    right_margin = img_w - left_margin
    if x < left_margin or x + w > right_margin:
        return False
    
    # Question region tolerance
    if question_region:
        TOLERANCE = 20
        if x < max(0, question_region.x - TOLERANCE):
            return False
        if y < max(0, question_region.y - TOLERANCE):
            return False
        if x + w > question_region.x + question_region.w + TOLERANCE:
            return False
        if y + h > question_region.y + question_region.h + TOLERANCE:
            return False
    
    # Area fraction
    area_frac = (w * h) / (img_w * img_h)
    if area_frac > MAX_AREA_FRAC:
        return False
    
    return True


# Tests
def test_expand_upward_for_above_caption():
    bbox = PixelRect(200, 200, 300, 150)
    hint = CaptionHint(0.2, 0.15, True)  # Above
    result = expand_bbox_for_caption(bbox, 1000, 1000, hint)
    assert result.y < bbox.y  # Expanded upward
    assert result.h > bbox.h
    print("  ok: expand_upward_for_above_caption")


def test_expand_downward_for_below_caption():
    bbox = PixelRect(200, 200, 300, 150)
    hint = CaptionHint(0.2, 0.4, False)  # Below
    result = expand_bbox_for_caption(bbox, 1000, 1000, hint)
    assert result.h > bbox.h  # Expanded downward
    print("  ok: expand_downward_for_below_caption")


def test_no_expansion_without_hint():
    bbox = PixelRect(200, 200, 300, 150)
    result = expand_bbox_for_caption(bbox, 1000, 1000, None)
    assert result.x == bbox.x and result.y == bbox.y and result.w == bbox.w and result.h == bbox.h
    print("  ok: no_expansion_without_hint")


def test_reject_footer_zone():
    # Box at y=870, h=50 (ends at 920, exactly at footer boundary)
    # Try to expand down toward footer - should be rejected
    bbox = PixelRect(200, 870, 300, 50)
    hint = CaptionHint(0.2, 0.95, False)  # Caption far below
    result = expand_bbox_for_caption(bbox, 1000, 1000, hint)
    # Should not expand because it would hit footer zone
    assert result.h == bbox.h  # No expansion
    print("  ok: reject_footer_zone")


def test_reject_header_zone():
    bbox = PixelRect(200, 100, 300, 150)  # Near top
    hint = CaptionHint(0.2, 0.02, True)
    result = expand_bbox_for_caption(bbox, 1000, 1000, hint)
    # Should not expand into header (top 5% = 50)
    assert result.y >= 50
    print("  ok: reject_header_zone")


def test_reject_side_margins():
    # Box at x=20, w=300, caption at x=10 tries to expand left into margin
    # Should be rejected, box stays at original position
    bbox = PixelRect(20, 200, 300, 150)
    hint = CaptionHint(0.01, 0.3, True)
    result = expand_bbox_for_caption(bbox, 1000, 1000, hint)
    assert result.x == bbox.x  # No left expansion
    assert result.w == bbox.w  # No width change
    print("  ok: reject_side_margins")


def test_respect_question_region():
    bbox = PixelRect(200, 200, 300, 150)
    hint = CaptionHint(0.1, 0.15, True)
    qr = PixelRect(150, 150, 400, 300)
    result = expand_bbox_for_caption(bbox, 1000, 1000, hint, qr)
    assert result.x >= 130  # qr.x - 20
    assert result.y >= 130  # qr.y - 20
    print("  ok: respect_question_region")


def test_bounded_expansion():
    bbox = PixelRect(200, 200, 300, 150)
    hint = CaptionHint(0.05, 0.05, True)  # Far above
    result = expand_bbox_for_caption(bbox, 1000, 1000, hint)
    # Max expansion is 15% of height or 80px
    max_exp = min(round(150 * 0.15), 80)
    assert bbox.y - result.y <= max_exp
    print("  ok: bounded_expansion")


if __name__ == "__main__":
    test_expand_upward_for_above_caption()
    test_expand_downward_for_below_caption()
    test_no_expansion_without_hint()
    test_reject_footer_zone()
    test_reject_header_zone()
    test_reject_side_margins()
    test_respect_question_region()
    test_bounded_expansion()
    print("\nALL GEOMETRY MIRROR TESTS PASSED")
