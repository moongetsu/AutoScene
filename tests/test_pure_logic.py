"""Tests for the new pure-logic functions added during the quality pass.

These functions are all pure (no side effects, no GPU/IO), so they can run
without InsightFace, TransNetV2, or ffmpeg installed.
"""
from __future__ import annotations

import math

import numpy as np
import pytest

from autoscene.faces.base import FaceDetection, pose_weight_from_kps


# ── pose_weight_from_kps ────────────────────────────────────────────────

class TestPoseWeight:
    def test_none_returns_full(self):
        assert pose_weight_from_kps(None) == 1.0

    def test_too_few_points(self):
        assert pose_weight_from_kps([(10, 20), (30, 40)]) == 1.0

    def test_frontal_near_full(self):
        # Eyes at (0,0)-(100,0), nose at (50,40) → almost perfect frontal
        kps = [(0, 0), (100, 0), (50, 40), (40, 70), (60, 70)]
        w = pose_weight_from_kps(kps)
        assert 0.95 <= w <= 1.0

    def test_profile_penalised(self):
        # Nose way off to the right → profile
        kps = [(0, 0), (100, 0), (90, 40), (40, 70), (60, 70)]
        w = pose_weight_from_kps(kps)
        assert 0.3 <= w < 0.85

    def test_neutral_kps_string(self):
        # Invalid input gracefully returns full weight
        assert pose_weight_from_kps("bad") == 1.0

    def test_eyes_coincident(self):
        # Degenerate: both eyes at same spot → eye_dist ~0
        kps = [(50, 50), (50, 50), (50, 80), (40, 90), (60, 90)]
        w = pose_weight_from_kps(kps)
        assert w == 1.0

    def test_bounds_always_in_03_to_1(self):
        # Stress: random kps should always be in [0.3, 1.0]
        rng = np.random.default_rng(42)
        for _ in range(200):
            raw = rng.uniform(0, 200, (5, 2))
            w = pose_weight_from_kps([(float(x), float(y)) for x, y in raw])
            assert 0.3 <= w <= 1.0


# ── merge_cut_windows ───────────────────────────────────────────────────

class TestEvenDownscale:
    def test_4k_to_480(self):
        from autoscene.scene_cuts import _even_downscale
        assert _even_downscale(3840, 2160) == (480, 270)

    def test_already_small(self):
        from autoscene.scene_cuts import _even_downscale
        assert _even_downscale(320, 180) == (320, 180)


class TestMergeCutWindows:
    def test_empty(self):
        from autoscene.scene_cuts import merge_cut_windows
        assert merge_cut_windows([], 0.3) == []

    def test_single(self):
        from autoscene.scene_cuts import merge_cut_windows
        r = merge_cut_windows([5.0], 0.3)
        assert len(r) == 1
        a, b = r[0]
        assert a == pytest.approx(4.7, abs=1e-6)
        assert b == pytest.approx(5.3, abs=1e-6)

    def test_merge_close(self):
        from autoscene.scene_cuts import merge_cut_windows
        # Two cuts within 0.4s of each other (pad=0.3 → overlap)
        r = merge_cut_windows([5.0, 5.2], 0.3)
        assert len(r) == 1
        assert r[0][0] == pytest.approx(4.7, abs=1e-6)
        assert r[0][1] == pytest.approx(5.5, abs=1e-6)

    def test_separate(self):
        from autoscene.scene_cuts import merge_cut_windows
        r = merge_cut_windows([1.0, 10.0], 0.3)
        assert len(r) == 2

    def test_negative_clamped(self):
        from autoscene.scene_cuts import merge_cut_windows
        r = merge_cut_windows([0.1], 0.5)
        assert r[0][0] == 0.0


# ── parse_scene_score_lines ─────────────────────────────────────────────

class TestParseSceneScore:
    def test_empty(self):
        from autoscene.scene_cuts import parse_scene_score_lines
        assert parse_scene_score_lines("") == []

    def test_valid_lines(self):
        from autoscene.scene_cuts import parse_scene_score_lines
        text = (
            "frame:100 pts:4170 pts_time:1.25 lavfi.scene_score=0.82\n"
            "frame:101 pts:4211 pts_time:1.26 lavfi.scene_score=0.04\n"
        )
        out = parse_scene_score_lines(text)
        assert len(out) == 2
        assert out[0] == (pytest.approx(1.25), pytest.approx(0.82))
        assert out[1] == (pytest.approx(1.26), pytest.approx(0.04))

    def test_garbage_skipped(self):
        from autoscene.scene_cuts import parse_scene_score_lines
        out = parse_scene_score_lines("not a metadata line at all")
        assert out == []

    def test_mixed(self):
        from autoscene.scene_cuts import parse_scene_score_lines
        text = (
            "frame:50 pts:2085 pts_time:0.83 lavfi.scene_score=0.12\n"
            "garbage line\n"
            "frame:60 pts:2502 pts_time:1.00 lavfi.scene_score=0.91\n"
        )
        out = parse_scene_score_lines(text)
        assert len(out) == 2


# ── pick_cut_peaks ──────────────────────────────────────────────────────

class TestPickCutPeaks:
    def test_empty_scores_returns_input(self):
        from autoscene.scene_cuts import pick_cut_peaks
        r = pick_cut_peaks([1.0, 2.0], [], pad_sec=0.3)
        assert r == [1.0, 2.0]

    def test_snaps_to_peak(self):
        from autoscene.scene_cuts import pick_cut_peaks
        scores = [
            (0.5, 0.10), (0.6, 0.10), (0.7, 0.85),
            (0.8, 0.10), (0.9, 0.10),
        ]
        r = pick_cut_peaks([0.65], scores, pad_sec=0.3, min_score=0.35)
        assert r[0] == pytest.approx(0.7)

    def test_weak_peak_keeps_original(self):
        from autoscene.scene_cuts import pick_cut_peaks
        scores = [
            (0.5, 0.05), (0.6, 0.08), (0.7, 0.12),
            (0.8, 0.08), (0.9, 0.05),
        ]
        r = pick_cut_peaks([0.65], scores, pad_sec=0.3, min_score=0.35)
        assert r[0] == pytest.approx(0.65)

    def test_multiple(self):
        from autoscene.scene_cuts import pick_cut_peaks
        scores = [
            (1.0, 0.1), (1.1, 0.9), (1.2, 0.1),
            (3.0, 0.1), (3.1, 0.1), (3.2, 0.85),
        ]
        r = pick_cut_peaks([1.1, 3.1], scores, pad_sec=0.3, min_score=0.35)
        assert r[0] == pytest.approx(1.1)
        assert r[1] == pytest.approx(3.2)


# ── weighted_centroid ───────────────────────────────────────────────────

class TestWeightedCentroid:
    def test_unweighted_mean(self):
        from autoscene.cluster import weighted_centroid
        rows = np.array([[1.0, 0.0], [0.0, 1.0], [1.0, 0.0]])
        c = weighted_centroid(rows)
        assert c.shape == (2,)
        n = float(np.linalg.norm(c))
        assert n == pytest.approx(1.0, abs=1e-6)

    def test_weighted_shifts(self):
        from autoscene.cluster import weighted_centroid
        rows = np.array([[1.0, 0.0], [0.0, 1.0]])
        w = np.array([10.0, 0.1])
        c_w = weighted_centroid(rows, w)
        c_u = weighted_centroid(rows)
        # Weighted should be closer to [1,0]
        assert abs(c_w[0]) > abs(c_u[0])

    def test_empty(self):
        from autoscene.cluster import weighted_centroid
        c = weighted_centroid(np.zeros((0, 512)))
        assert c.size == 0


# ── detection_identity_weights ──────────────────────────────────────────

class TestDetectionIdentityWeights:
    def test_basic(self):
        from autoscene.cluster import detection_identity_weights
        d1 = FaceDetection(
            timestamp_sec=0.0, bbox=(0, 0, 10, 10),
            embedding=np.zeros(512), det_score=0.9, pose_weight=1.0,
        )
        d2 = FaceDetection(
            timestamp_sec=0.1, bbox=(0, 0, 10, 10),
            embedding=np.zeros(512), det_score=0.5, pose_weight=0.4,
        )
        w = detection_identity_weights([d1, d2], [0, 1])
        assert w[0] == pytest.approx(0.9 * 1.0)
        assert w[1] == pytest.approx(0.5 * 0.4)


# ── _shots_of_segment ──────────────────────────────────────────────────

class TestShotsOfSegment:
    def _make_seg(self, start, end):
        class Seg:
            start_sec = start
            end_sec = end
        return Seg()

    def test_full_coverage(self):
        from autoscene.pack_context import _shots_of_segment
        starts = [0.0, 2.0, 4.0]
        ends   = [2.0, 4.0, 6.0]
        seg = self._make_seg(0.0, 6.0)
        r = _shots_of_segment(starts, ends, seg)
        assert r == [0, 1, 2]

    def test_partial(self):
        from autoscene.pack_context import _shots_of_segment
        starts = [0.0, 2.0, 4.0]
        ends   = [2.0, 4.0, 6.0]
        seg = self._make_seg(1.5, 3.5)
        r = _shots_of_segment(starts, ends, seg)
        # Should include shot 0 (start inside), shot 1 (fully inside)
        assert 0 in r and 1 in r

    def test_empty_shots(self):
        from autoscene.pack_context import _shots_of_segment
        r = _shots_of_segment([], [], self._make_seg(0, 1))
        assert r == []


# ── _shots_from_cuts ───────────────────────────────────────────────────

class TestShotsFromCuts:
    def test_no_cuts(self):
        from autoscene.scene_cuts import _shots_from_cuts
        shots = _shots_from_cuts([], 10.0)
        assert len(shots) == 1
        assert shots[0].start_sec == 0.0
        assert shots[0].end_sec == 10.0

    def test_single_cut(self):
        from autoscene.scene_cuts import _shots_from_cuts
        shots = _shots_from_cuts([5.0], 10.0)
        assert len(shots) >= 2
        assert shots[0].start_sec == 0.0
        assert shots[-1].end_sec == 10.0

    def test_merges_close_cuts(self):
        from autoscene.scene_cuts import _shots_from_cuts
        # Two cuts within 0.12s → merged into one boundary
        shots = _shots_from_cuts([5.0, 5.05], 10.0)
        # Should still produce at least 1 shot spanning full duration
        assert len(shots) >= 1
        assert shots[0].start_sec == 0.0


# ── adjacent shot merge in build_segments ───────────────────────────────

class TestBuildSegmentsAdjacentMerge:
    def _make_det(self, ts, pid=0):
        return FaceDetection(
            timestamp_sec=ts,
            bbox=(0, 0, 100, 100),
            embedding=np.random.default_rng(0).normal(size=512),
            det_score=0.9,
        )

    def test_consecutive_shots_merge(self):
        from autoscene.cluster import Segment, build_segments
        from autoscene.scene_cuts import ShotSpan

        shots = [ShotSpan(0.0, 2.0), ShotSpan(2.0, 4.0), ShotSpan(4.0, 6.0)]
        dets = [
            self._make_det(0.5),  # shot 0
            self._make_det(1.5),  # shot 0
            self._make_det(2.5),  # shot 1
            self._make_det(3.5),  # shot 1
            self._make_det(4.5),  # shot 2
            self._make_det(5.5),  # shot 2
        ]
        labels = np.array([0, 0, 0, 0, 0, 0])
        segs = build_segments(dets, labels, shots=shots, min_hits_per_shot=2)
        # All 3 shots are consecutive → should be ONE merged segment
        assert len(segs) == 1
        assert segs[0].start_sec == pytest.approx(0.0)
        assert segs[0].end_sec == pytest.approx(6.0)

    def test_gap_breaks_segment(self):
        from autoscene.cluster import build_segments
        from autoscene.scene_cuts import ShotSpan

        shots = [ShotSpan(0.0, 2.0), ShotSpan(2.0, 4.0), ShotSpan(4.0, 6.0)]
        dets = [
            self._make_det(0.5),  # shot 0
            self._make_det(1.5),  # shot 0
            # shot 1: no detections → gap
            self._make_det(4.5),  # shot 2
            self._make_det(5.5),  # shot 2
        ]
        labels = np.array([0, 0, 0, 0])
        segs = build_segments(dets, labels, shots=shots, min_hits_per_shot=2)
        # Two separate segments (shot 0 + shot 2)
        assert len(segs) == 2
        starts = sorted(s.start_sec for s in segs)
        assert starts[0] == pytest.approx(0.0)
        assert starts[1] == pytest.approx(4.0)
