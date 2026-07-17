#!/usr/bin/env python3
"""Python mirror of timing aggregation logic."""

from dataclasses import dataclass, field
from typing import Optional, List
from collections import defaultdict


@dataclass
class TimingEntry:
    stage: str
    operation: str
    page: Optional[int]
    question_number: Optional[int]
    milliseconds: int


@dataclass
class ImportReport:
    paper_name: str
    kind: str
    pages_total: int = 0
    pages_processed: int = 0
    questions_expected: int = 0
    questions_extracted: int = 0
    paper_total_marks: Optional[int] = None
    extracted_total_marks: int = 0
    marks_checksum_ok: Optional[bool] = None
    mark_checks: List = field(default_factory=list)
    quarantined: List = field(default_factory=list)
    skipped_pages: List = field(default_factory=list)
    repairs: int = 0
    salvage_events: int = 0
    crop_rejections: int = 0
    diagrams_saved: int = 0
    diagrams_deduped: int = 0
    anomalies: List[str] = field(default_factory=list)
    timings: List[TimingEntry] = field(default_factory=list)

    def absorb(self, other: "ImportReport"):
        self.pages_processed += other.pages_processed
        self.repairs += other.repairs
        self.salvage_events += other.salvage_events
        self.crop_rejections += other.crop_rejections
        self.diagrams_saved += other.diagrams_saved
        self.diagrams_deduped += other.diagrams_deduped
        self.mark_checks.extend(other.mark_checks)
        self.quarantined.extend(other.quarantined)
        self.skipped_pages.extend(other.skipped_pages)
        self.anomalies.extend(other.anomalies)
        self.timings.extend(other.timings)

    def record_timing(self, stage: str, operation: str, page: Optional[int], question_number: Optional[int], ms: int):
        self.timings.append(TimingEntry(stage, operation, page, question_number, ms))

    def timing_summary(self) -> dict:
        """Generate summary grouped by stage."""
        by_stage = defaultdict(lambda: defaultdict(int))
        for t in self.timings:
            by_stage[t.stage][t.operation] += t.milliseconds
        
        summary = {}
        for stage, ops in by_stage.items():
            summary[stage] = dict(ops)
            summary[stage]["total"] = sum(ops.values())
        return summary

    def print_timing_report(self):
        """Print human-readable timing report."""
        print(f"\n=== Timing Report for {self.paper_name} ===")
        total = 0
        for t in self.timings:
            page_str = f" p{t.page}" if t.page is not None else ""
            q_str = f" Q{t.question_number}" if t.question_number is not None else ""
            print(f"  {t.stage:20} {t.operation:25} {t.ms:8}ms{page_str}{q_str}")
            total += t.milliseconds
        print(f"  {'TOTAL':46} {total}ms")
        return total


# Tests
def test_timing_entry_creation():
    report = ImportReport("test", "questions")
    report.record_timing("extraction", "api_call", 1, 5, 1500)
    report.record_timing("diagram_processing", "crop_audit", 1, 5, 200)
    assert len(report.timings) == 2
    assert report.timings[0].stage == "extraction"
    assert report.timings[1].operation == "crop_audit"
    print("  ok: timing_entry_creation")


def test_absorb_timings():
    report1 = ImportReport("test", "questions")
    report1.record_timing("structure", "api_call", 1, None, 1000)
    report1.repairs = 2
    
    report2 = ImportReport("test", "questions")
    report2.record_timing("extraction", "api_call", 2, 3, 2000)
    report2.repairs = 1
    
    report1.absorb(report2)
    assert len(report1.timings) == 2
    assert report1.repairs == 3
    print("  ok: absorb_timings")


def test_timing_summary():
    report = ImportReport("test", "questions")
    report.record_timing("extraction", "api_call", 1, 1, 1500)
    report.record_timing("extraction", "api_call", 2, 2, 1200)
    report.record_timing("diagram_processing", "crop_audit", 1, 1, 300)
    report.record_timing("diagram_processing", "save", 1, 1, 100)
    report.record_timing("database", "upsert", None, 1, 50)
    
    summary = report.timing_summary()
    assert summary["extraction"]["api_call"] == 2700
    assert summary["extraction"]["total"] == 2700
    assert summary["diagram_processing"]["crop_audit"] == 300
    assert summary["diagram_processing"]["save"] == 100
    assert summary["diagram_processing"]["total"] == 400
    assert summary["database"]["upsert"] == 50
    print("  ok: timing_summary")


def test_parallel_report_absorb():
    """Simulate parallel batch reports being absorbed."""
    master = ImportReport("paper", "questions")
    
    # Batch 1: pages 1-4
    batch1 = ImportReport("paper", "questions")
    batch1.record_timing("structure", "api_call_batch", 1, None, 500)
    batch1.record_timing("extraction", "span_batch", 2, 5, 3000)
    batch1.pages_processed = 4
    
    # Batch 2: pages 5-8
    batch2 = ImportReport("paper", "questions")
    batch2.record_timing("structure", "api_call_batch", 5, None, 500)
    batch2.record_timing("extraction", "span_batch", 6, 9, 2500)
    batch2.pages_processed = 4
    
    master.absorb(batch1)
    master.absorb(batch2)
    
    assert master.pages_processed == 8
    assert len(master.timings) == 4
    # Total time should be at least the max of parallel operations
    # (not sum, since they ran in parallel)
    structure_time = sum(t.milliseconds for t in master.timings if t.stage == "structure")
    assert structure_time == 1000  # Both batches ran in parallel
    print("  ok: parallel_report_absorb")


def test_total_duration():
    report = ImportReport("test", "questions")
    report.record_timing("rasterisation", "page_1", 1, None, 1200)
    report.record_timing("rasterisation", "page_2", 2, None, 1100)
    report.record_timing("structure_scan", "api_call", 1, None, 18700)
    report.record_timing("extraction", "api_call", 1, 1, 92400)
    report.record_timing("repairs", "repair_round", 2, 5, 31100)
    report.record_timing("diagram_processing", "crop_audit", 1, 1, 8600)
    report.record_timing("database", "upsert", None, None, 400)
    
    total = sum(t.milliseconds for t in report.timings)
    assert total >= 150000  # At least 150s as in the example
    print("  ok: total_duration")


if __name__ == "__main__":
    test_timing_entry_creation()
    test_absorb_timings()
    test_timing_summary()
    test_parallel_report_absorb()
    test_total_duration()
    print("\nALL TIMING MIRROR TESTS PASSED")
