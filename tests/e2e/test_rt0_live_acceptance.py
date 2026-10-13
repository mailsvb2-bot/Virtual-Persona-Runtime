"""Non-promoting live RT0 acceptance handoff test matrix."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
from rt0_live_acceptance import InvalidEvidence, build_handoff, render_markdown

CANDIDATE = 'a' * 40
PROVIDER = 'b' * 64

def sample(role='owner'):
    return {
        'schema_version': 'rt0-owner-lab-session-evidence-1.6',
        'candidate_sha': CANDIDATE,
        'provider_state_sha256': PROVIDER,
        'scope': 'browser_observed_media_plane_only',
        'participant_role': role,
        'session_sequence': 1,
        'session_duration_millis': 30000,
        'canonical_playback_proven': False,
        'av_sync_proven': False,
        'voice_attempts': [{
            'request_sequence': 1, 'status': 'completed',
            'stt_millis': 230, 'llm_millis': 1680,
            'canonical_playback_confirmed': False,
        }],
        'media_events': [
            {'kind': 'end_to_end_video_ready', 'request_sequence': None, 'elapsed_millis': 6500},
            {'kind': 'audio_started', 'request_sequence': 1, 'elapsed_millis': 2700},
            {'kind': 'interruption_stopped', 'request_sequence': 1, 'elapsed_millis': 85},
        ],
        'av_sync_samples': [],
        'av_sync_diagnostics': [{'audio_issue': 'timestamp_unavailable', 'video_issue': 'timestamp_unavailable'}],
        'private_secret': 'DO_NOT_ECHO_PRIVATE_OWNER_CONTENT',
    }

class HandoffMatrix(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def save(self, name, obj):
        path = self.root / name
        path.write_text(json.dumps(obj, ensure_ascii=False), encoding='utf-8')
        return path

    def gate(self, report, gate_id):
        return next(g['state'] for g in report['gates'] if g['id'] == gate_id)

    def test_real_owner_observation_is_not_promoted_to_rt0_exit(self):
        report = build_handoff([self.save('owner.json', sample())], 'rejected')
        self.assertFalse(report['release_ready_claimed'])
        self.assertEqual(report['disposition'], 'NOT_RELEASE_EVIDENCE')
        self.assertEqual(self.gate(report, 'owner_stop'), 'observed_not_exit_verified')
        self.assertEqual(self.gate(report, 'visitor_voice'), 'not_evidenced')
        self.assertEqual(self.gate(report, 'canonical_playback'), 'not_evidenced')
        self.assertEqual(self.gate(report, 'human_quality'), 'human_rejected')
        self.assertEqual(report['observations']['owner'][0]['observed_interrupt_stop_millis'], 85)
        self.assertNotIn('DO_NOT_ECHO', json.dumps(report))
        self.assertNotIn('DO_NOT_ECHO', render_markdown(report))

    def test_visitor_role_only_is_not_independent_human_proof(self):
        owner = sample()
        visitor = sample('visitor')
        visitor['session_sequence'] = 2
        visitor['media_events'].pop()
        report = build_handoff([self.save('owner.json', owner), self.save('visitor.json', visitor)], 'accepted')
        self.assertEqual(self.gate(report, 'visitor_voice'), 'observed_not_exit_verified')
        self.assertEqual(self.gate(report, 'visitor_video'), 'observed_not_exit_verified')
        self.assertEqual(self.gate(report, 'distinct_visitor'), 'not_evidenced')
        self.assertEqual(self.gate(report, 'human_quality'), 'not_evidenced')
        self.assertFalse(report['release_ready_claimed'])

    def test_identical_input_or_cross_candidate_is_rejected(self):
        o = self.save('one.json', sample())
        with self.assertRaisesRegex(InvalidEvidence, 'duplicate'):
            build_handoff([o, o])
        changed = sample('visitor')
        changed['candidate_sha'] = 'c' * 40
        v = self.save('visitor.json', changed)
        with self.assertRaisesRegex(InvalidEvidence, 'cross-candidate'):
            build_handoff([o, v])
        changed['candidate_sha'] = CANDIDATE
        changed['provider_state_sha256'] = 'd' * 64
        v = self.save('visitor.json', changed)
        with self.assertRaisesRegex(InvalidEvidence, 'cross-candidate'):
            build_handoff([o, v])

    def test_invalid_schema_role_media_and_proof_fail_closed(self):
        for field, value in [
            ('schema_version', 'rt0-owner-lab-session-evidence-1.5'),
            ('scope', 'synthetic'),
            ('participant_role', 'independent_human_visitor'),
            ('session_duration_millis', float('nan')),
            ('canonical_playback_proven', 'true'),
            ('av_sync_samples', None),
        ]:
            with self.subTest(field=field):
                s = sample()
                s[field] = value
                with self.assertRaises(InvalidEvidence):
                    build_handoff([self.save('bad.json', s)])
        s = sample()
        s['media_events'][0]['elapsed_millis'] = -1
        with self.assertRaises(InvalidEvidence):
            build_handoff([self.save('bad.json', s)])
        s = sample()
        s['voice_attempts'][0]['llm_millis'] = 'fast'
        with self.assertRaises(InvalidEvidence):
            build_handoff([self.save('bad.json', s)])

    def test_wrong_or_oversized_input_cannot_be_interpreted_as_evidence(self):
        with self.assertRaisesRegex(InvalidEvidence, 'at least one'):
            build_handoff([])
        p = self.root / 'bad.json'
        p.write_text('{broken', encoding='utf-8')
        with self.assertRaisesRegex(InvalidEvidence, 'invalid UTF-8 JSON'):
            build_handoff([p])
        p.write_bytes(b' ' * 2_000_001)
        with self.assertRaisesRegex(InvalidEvidence, 'bounded'):
            build_handoff([p])

    def test_absent_cost_and_sync_remain_unknown_even_with_fast_stop(self):
        s = sample()
        s['canonical_playback_proven'] = True
        s['av_sync_proven'] = True
        s['av_sync_samples'] = [{}, {}, {}]
        report = build_handoff([self.save('owner.json', s)])
        self.assertEqual(self.gate(report, 'canonical_playback'), 'not_evidenced')
        self.assertEqual(self.gate(report, 'av_sync'), 'not_evidenced')
        self.assertEqual(self.gate(report, 'billing'), 'not_evidenced')
        self.assertFalse(report['release_ready_claimed'])

    def test_video_before_any_voice_turn_is_a_session_event_not_a_voice_turn(self):
        s = sample()
        s['voice_attempts'] = []
        s['media_events'] = [
            {'kind': 'backend_start_ready', 'request_sequence': None, 'elapsed_millis': 4000},
            {'kind': 'end_to_end_video_ready', 'request_sequence': None, 'elapsed_millis': 6500},
            {'kind': 'reconnect_restored', 'request_sequence': None, 'elapsed_millis': 100},
        ]
        report = build_handoff([self.save('video-only.json', s)])
        self.assertEqual(self.gate(report, 'owner_video'), 'observed_not_exit_verified')
        self.assertEqual(self.gate(report, 'owner_voice'), 'not_evidenced')
        self.assertEqual(self.gate(report, 'owner_audio'), 'not_evidenced')
        self.assertEqual(self.gate(report, 'owner_stop'), 'not_evidenced')
        self.assertFalse(report['release_ready_claimed'])

    def test_media_must_respect_exact_rust_session_and_voice_request_scope(self):
        scenarios = [
            ('session_video_forged_voice', 0, 'request_sequence', 1),
            ('audio_without_voice_number', 1, 'request_sequence', None),
            ('audio_unknown_voice_number', 1, 'request_sequence', 999),
            ('stop_unknown_voice_number', 2, 'request_sequence', 999),
            ('unknown_media_kind', 1, 'kind', 'fabricated_success'),
            ('nonpositive_voice_number', 1, 'request_sequence', 0),
        ]
        for label, index, field, value in scenarios:
            with self.subTest(label=label):
                s = sample()
                s['media_events'][index][field] = value
                with self.assertRaises(InvalidEvidence):
                    build_handoff([self.save('bad.json', s)])

    def test_failed_or_missing_voice_attempt_cannot_create_playback_or_stop(self):
        for mode in ('failed', 'missing'):
            with self.subTest(mode=mode):
                s = sample()
                if mode == 'failed':
                    s['voice_attempts'][0]['status'] = 'failed'
                else:
                    s['voice_attempts'] = []
                with self.assertRaises(InvalidEvidence):
                    build_handoff([self.save('bad.json', s)])

    def test_duplicate_audio_and_unpaired_stop_fail_closed(self):
        s = sample()
        s['media_events'].append(dict(s['media_events'][1]))
        with self.assertRaisesRegex(InvalidEvidence, 'duplicate canonical'):
            build_handoff([self.save('bad.json', s)])
        s = sample()
        s['media_events'].pop(1)  # Stop without an AudioStarted event for turn 1.
        with self.assertRaisesRegex(InvalidEvidence, 'matching started audio'):
            build_handoff([self.save('bad.json', s)])

    def test_interruption_cannot_be_spliced_across_different_voice_requests(self):
        s = sample()
        s['voice_attempts'].append({
            'request_sequence': 2, 'status': 'completed',
            'stt_millis': 180, 'llm_millis': 1200,
            'canonical_playback_confirmed': False,
        })
        s['media_events'][2]['request_sequence'] = 2
        with self.assertRaisesRegex(InvalidEvidence, 'matching started audio'):
            build_handoff([self.save('bad.json', s)])
        s['media_events'].append({
            'kind': 'audio_started', 'request_sequence': 2,
            'elapsed_millis': 1900,
        })
        report = build_handoff([self.save('valid.json', s)])
        self.assertEqual(self.gate(report, 'owner_stop'), 'observed_not_exit_verified')
        self.assertFalse(report['release_ready_claimed'])

    def test_distinct_files_same_session_generation_are_not_independent(self):
        first = sample()
        second = sample('visitor')
        second['private_secret'] = 'different snapshot same session generation'
        a = self.save('owner.json', first)
        b = self.save('visitor.json', second)
        with self.assertRaisesRegex(InvalidEvidence, 'duplicate session sequence'):
            build_handoff([a, b])

if __name__ == '__main__':
    unittest.main()
