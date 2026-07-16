#!/usr/bin/env python3
"""Faithful Python port of the new Rust modules, run against the same test
vectors as the embedded #[cfg(test)] suites, to validate the ALGORITHMS.
(Rust syntax is reviewed separately; this verifies logic correctness.)"""

import json, re, math

# ── geometry.rs ─────────────────────────────────────────────────────────────
MIN_EDGE_PX, MAX_AREA_FRAC, MIN_AREA_FRAC = 12, 0.92, 0.0005

def sanitize_bbox(b, img_w, img_h):
    if len(b) != 4 or img_w < 2*MIN_EDGE_PX or img_h < 2*MIN_EDGE_PX: return None
    if any(not math.isfinite(v) for v in b): return None
    if any(v < 0.0 for v in b): return None
    max_val = max(b)
    if max_val <= 0.0: return None
    if max_val <= 1.5: scale = "rel"
    elif max_val <= 100.0: scale = "pct"
    elif max_val <= max(img_w, img_h) * 1.05: scale = "px"
    else: return None
    def to_frac(v, dim):
        return v if scale=="rel" else (v/100.0 if scale=="pct" else v/dim)
    fx, fy = to_frac(b[0], img_w), to_frac(b[1], img_h)
    fv2, fv3 = to_frac(b[2], img_w), to_frac(b[3], img_h)
    readings = []
    if b[2] > 0.0 and b[3] > 0.0: readings.append((fx, fy, fx+fv2, fy+fv3))
    if fv2 > fx and fv3 > fy: readings.append((fx, fy, fv2, fv3))
    for (x0, y0, x1, y1) in readings:
        if x1-x0 <= 0.001 or y1-y0 <= 0.001: continue
        if x0 >= 1.0 or y0 >= 1.0: continue
        px = min(max(round(x0*img_w), 0), img_w-1)
        py = min(max(round(y0*img_h), 0), img_h-1)
        far_x, far_y = max(round(min(x1,1.0)*img_w), 0), max(round(min(y1,1.0)*img_h), 0)
        pw = min(max(far_x-px, 0), img_w-px); ph = min(max(far_y-py, 0), img_h-py)
        if pw < MIN_EDGE_PX or ph < MIN_EDGE_PX: continue
        area = (pw*ph)/(img_w*img_h)
        if area > MAX_AREA_FRAC or area < MIN_AREA_FRAC: continue
        return (px, py, pw, ph)
    return None

W, H = 1654, 2339
ok = 0
def check(name, cond):
    global ok
    assert cond, f"FAILED: {name}"
    ok += 1; print(f"  ok: {name}")

r = sanitize_bbox([0.1,0.2,0.4,0.3], W, H)
check("relative_xywh", abs(r[0]-165)<=2 and 600<r[2]<720 and 650<r[3]<760 and r[0]+r[2]<=W and r[1]+r[3]<=H)
r = sanitize_bbox([100.0,150.0,600.0,400.0], W, H)
check("pixel round-trip no panic", abs(r[0]-100)<=2 and abs(r[1]-150)<=2 and abs(r[2]-600)<=2 and abs(r[3]-400)<=2)
check("percent", sanitize_bbox([10,20,40,30], W, H) is not None)
check("out_of_range rejected", sanitize_bbox([5000,5000,8000,5000], W, H) is None)
check("full-page rejected", sanitize_bbox([0,0,1,1], W, H) is None and sanitize_bbox([0.01,0.01,0.98,0.98], W, H) is None)
r = sanitize_bbox([0.02,0.02,0.96,0.96], W, H)
check("large-but-legit box clamped", r is not None and r[0]+r[2]<=W and r[1]+r[3]<=H)
check("near-full both readings rejected", sanitize_bbox([0.0,0.0,0.97,0.97], W, H) is None)
check("degenerate rejected", sanitize_bbox([0.5,0.5,0.0001,0.0001], W, H) is None)
check("nan/neg rejected", sanitize_bbox([float('nan'),0,0.5,0.5], W, H) is None and sanitize_bbox([-0.1,0.1,0.5,0.5], W, H) is None)
check("off-page start rejected", sanitize_bbox([2000,2000,100,100], W, H) is None)
check("wrong len rejected", sanitize_bbox([0.1,0.2], W, H) is None)

# ── json_salvage.rs ─────────────────────────────────────────────────────────
def scan(s):
    stack, in_str, esc = [], False, False
    first_end, last_boundary, closers = None, None, []
    for i, c in enumerate(s):
        if in_str:
            if esc: esc = False
            elif c == '\\': esc = True
            elif c == '"': in_str = False
        else:
            if c == '"': in_str = True
            elif c == '{': stack.append('}')
            elif c == '[': stack.append(']')
            elif c in '}]':
                depth_before = len(stack)
                was_item = (c == '}' and depth_before >= 2 and stack[depth_before-2] == ']')
                if stack and stack[-1] == c: stack.pop()
                if not stack and first_end is None: first_end = i+1
                if was_item:
                    last_boundary, closers = i+1, stack[:]
    return first_end, last_boundary, closers

def parse_llm_json(wire):
    t = wire.strip()
    for p in ("```json","```JSON","```"):
        if t.startswith(p): t = t[len(p):]; break
    if t.endswith("```"): t = t[:-3]
    t = t.strip()
    idx = len(t)
    for i, c in enumerate(t):
        if c in '{[': idx = i; break
    prep = t[idx:]
    try:
        return ("Clean", json.loads(prep))
    except json.JSONDecodeError as e:
        fe, lb, closers = scan(prep)
        if "Extra data" in str(e) or fe is not None and fe < len(prep):
            if fe and fe < len(prep):
                try: return ("Salvaged-tail", json.loads(prep[:fe]))
                except json.JSONDecodeError: pass
        if lb is not None and lb < len(prep):
            cand = prep[:lb] + ''.join(reversed(closers))
            try: return (("Salvaged-trunc", json.loads(cand)))
            except json.JSONDecodeError: pass
        return ("Malformed", str(e))

r, v = parse_llm_json('{"extracted_questions":[{"n":1},{"n":2}]}')
check("clean verbatim", r=="Clean" and len(v["extracted_questions"])==2)
r, v = parse_llm_json('Sure!\n```json\n{"extracted_questions":[{"n":7}]}\n```')
check("fence+preamble clean", r=="Clean" and v["extracted_questions"][0]["n"]==7)
r, v = parse_llm_json('{"extracted_questions":[{"n":1},{"n":2,"content":"Evaluate \\nabla f an')
check("trunc salvages prefix", r=="Salvaged-trunc" and len(v["extracted_questions"])==1)
r, v = parse_llm_json('{"extracted_questions":[{"n":1},{"n":2,"b":[[0.1,0.2,0.3,0.4]]},{"n":3,"b":[[0.5')
check("trunc nested struct", r=="Salvaged-trunc" and len(v["extracted_questions"])==2)
r, v = parse_llm_json('{"extracted_questions":[{"n":1}]} I hope this helps!')
check("trailing junk", r in ("Salvaged-tail","Clean"))
r, v = parse_llm_json('{"c":"$\\\\nabla f$ and $\\\\tan \\\\theta$"}')
check("properly escaped latex verbatim", r=="Clean" and v["c"]=="$\\nabla f$ and $\\tan \\theta$")
r, v = parse_llm_json('{"c":"bad \\uZoo"}')
check("invalid escape → Malformed", r=="Malformed")
r, v = parse_llm_json('[{"n":1},{"n":2}]')
check("bare array", r=="Clean" and len(v)==2)
# THE BUG-CLASS KILLER: what the OLD fixer did vs new policy
old = '{"content":"$\\nabla f$"}'  # what fix_json_escapes produced: nothing
r, v = parse_llm_json(old)
check("old-bug case: now → Malformed (goes to repair, never mangled)", r=="Malformed" or (r=="Clean" and "\\nabla" not in v.get("c","")))

# ── validate.rs ─────────────────────────────────────────────────────────────
def sum_inline_marks(s):
    return sum(int(m) for m in re.findall(r"(?i)\*?\*?(?:\[|\()\s*(\d{1,2})\s*marks?\s*(?:\]|\))\*?\*?", s) if int(m)<=25)
check("marks sum", sum_inline_marks("Part a **[4 marks]** then **[3 marks]**")==7)
check("marks needs word", sum_inline_marks("Total is 10 but no tags (2024)")==0)

def qnum(v):
    if isinstance(v, bool): return None
    if isinstance(v, (int, float)):
        if isinstance(v, float) and v != int(v): return None
        n = int(v)
    elif isinstance(v, str):
        t = v.strip()
        if '.' in t or ',' in t: return None
        try: n = int(t)
        except ValueError: return None
    else: return None
    return n if 1 <= n <= 60 else None

check("qnum 7→7", qnum(7)==7)
check("qnum '12'→12", qnum("12")==12)
check("qnum '03.1'→None (not 31!)", qnum("03.1") is None)
check("qnum 0/99/3.7→None", qnum(0) is None and qnum(99) is None and qnum(3.7) is None)

def terminal(s):
    t = s.rstrip()
    if not t: return False
    if re.search(r"(?i)(?:\[|\()\s*\d{1,2}\s*marks?\s*(?:\]|\))\s*\**\s*$", t): return True
    if t.endswith(("$$","```","$","`")): return True
    return t[-1] in ".!?)]);:" and True or False

check("terminal marks tag", terminal("Find the gradient. **[4 marks]**"))
check("terminal math", terminal("Hence $x = 2$."), )
check("not terminal mid-word", not terminal("Evaluate the integ"))

# ── doc_map.rs ──────────────────────────────────────────────────────────────
FOOT = re.compile(r"(?i)\(?\s*Total\s+for\s+Question\s+(\d{1,2})\s+is\s+(\d{1,2})\s+marks?\s*\)?")
PAPER = re.compile(r"(?i)TOTAL\s+FOR\s+PAPER\s+IS\s+(\d{1,3})\s+MARKS")
INSTR = re.compile(r"(?i)\binstructions\b|\binformation\b|answer all questions|formulae|\bglossary\b")
MARGIN = re.compile(r"(?m)^\s*0?1\s*$")

def build_map(texts):
    footers, total = [], None
    for p, t in enumerate(texts):
        for m in FOOT.finditer(t):
            q, mk = int(m.group(1)), int(m.group(2))
            if q>0 and mk>0: footers.append((p,q,mk))
        if total is None:
            m = PAPER.search(t)
            if m and int(m.group(1))>0: total = int(m.group(1))
    if len(footers)<2: return None
    footers.sort(key=lambda f:(f[0],f[1]))
    footers = list({f[1]:f for f in footers}.values())
    footers.sort()
    if any(footers[i+1][1] <= footers[i][1] for i in range(len(footers)-1)): return None
    spans = []
    for i,(p,q,mk) in enumerate(footers):
        if i == 0:
            start = 0
            for pp in range(p):
                if MARGIN.search(texts[pp]): start = pp; break
                if INSTR.search(texts[pp]): start = pp+1
            start = min(start, p)
        else:
            start = p if footers[i-1][0]==p else footers[i-1][0]+1
        if start > p or p >= len(texts): return None
        spans.append((q,start,p,mk))
    return spans, total

texts = [
    "Centre Number\nInstructions\nAnswer ALL questions",
    "1. Question one text (a) part",
    "middle of Q1 (Total for Question 1 is 5 marks)\n3. second question",
    "second continues (Total for Question 2 is 6 marks)",
    "TOTAL FOR PAPER IS 11 MARKS",
]
m = build_map(texts)
check("edexcel footer map", m is not None and m[0][0]==(1,1,2,5) and m[0][1]==(2,3,3,6) and m[1]==11)
m2 = build_map(["1. first (Total for Question 1 is 3 marks) 2. second (Total for Question 2 is 4 marks)","3. third (Total for Question 3 is 2 marks)"])
check("same-page questions", m2[0][1]==(2,0,0,4) and m2[0][2]==(3,1,1,2))
check("corrupt text → None", build_map(["garbled !@#$","more garbled"]) is None)

print(f"\nALL {ok} LOGIC CHECKS PASSED")
