#!/usr/bin/env python3
"""Non-promoting RT0 live-session evidence handoff.

Only typed, sanitized browser evidence is admitted. This diagnostic is NOT the
canonical RT0 exit evaluator, and never produces passed/released claims.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
from typing import Any

SCHEMA = 'rt0-owner-lab-session-evidence-1.6'
OUTPUT_SCHEMA = 'rt0-live-acceptance-handoff-0.1'
MAX_INPUT_BYTES = 2_000_000
GATES = (
    ('owner_voice', 'Real owner completed Russian voice turn'),
    ('visitor_voice', 'Independent non-owner completed voice turn'),
    ('owner_stop', 'Owner interrupted audible speech'),
    ('owner_video', 'Owner received real rendered avatar video'),
    ('visitor_video', 'Visitor received real rendered avatar video'),
    ('owner_audio', 'Owner browser observed first audio'),
    ('visitor_audio', 'Visitor browser observed first audio'),
    ('canonical_playback', 'Canonical provider-confirmed playback for owner and visitor'),
    ('av_sync', 'Required provider-grounded three-sample A/V synchronization'),
    ('reconnect', 'Verified recoverable real provider reconnect'),
    ('claim_correction', 'Corrected owner claim applied on next real turn'),
    ('revocation_deny', 'Real revoke followed by denied provider egress'),
    ('billing', 'Measured and reviewed total provider costs'),
    ('golden', 'Exact-candidate Golden evaluation'),
    ('privacy', 'Reviewed permissions and attribution acceptance'),
    ('human_quality', 'Five-dimension human acceptance including natural Russian voice'),
    ('distinct_visitor', 'Independent human visitor provenance attested'),
)


# Match vpr_evaluation::session_evidence::LabMediaEvidenceKind and
# requires_request_sequence(). Avatar connect and first video are SESSION
# events, while playback, provider and STOP diagnostics are VOICE TURN events.
# Never infer a voice turn from a filename, timing value or UI-only event.
SESSION_MEDIA_KINDS = frozenset({
    'video_ready',
    'backend_start_ready',
    'transport_connect_started',
    'transport_connected',
    'remote_video_track_received',
    'remote_video_attached',
    'end_to_end_video_ready',
    'reconnect_restored',
})
VOICE_MEDIA_KINDS = frozenset({
    'backend_complete_received',
    'client_delivery_sent',
    'audio_started',
    'provider_data_received',
    'provider_event_ignored',
    'provider_unknown_chat_event',
    'provider_unknown_video_event',
    'provider_unknown_tool_event',
    'provider_unknown_other_event',
    'provider_video_generation_started',
    'provider_video_generation_done',
    'provider_video_generation_failed',
    'provider_informational_event',
    'provider_event_parse_failed',
    'provider_playback_done_received',
    'playback_recovery_triggered',
    'browser_audio_tail_observed',
    'playback_completed',
    'interruption_stopped',
})

class InvalidEvidence(ValueError):
    """Malformed, unsupported, stale, or conflicting evidence."""

def _nonnegative(value: Any, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise InvalidEvidence(f'invalid {label}')
    return float(value)

def _positive_sequence(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise InvalidEvidence(f'invalid {label}')
    return value

def _required_digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
        raise InvalidEvidence(f'invalid {label}')
    return value

def _read(path: Path) -> tuple[dict[str, Any], str]:
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise InvalidEvidence('evidence must be a bounded regular JSON file')
    raw = path.read_bytes()
    if len(raw) > MAX_INPUT_BYTES:
        raise InvalidEvidence('evidence exceeds size limit')
    try:
        item = json.loads(raw.decode('utf-8'))
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise InvalidEvidence('invalid UTF-8 JSON evidence') from exc
    if not isinstance(item, dict) or item.get('schema_version') != SCHEMA:
        raise InvalidEvidence('unsupported session schema')
    candidate = item.get('candidate_sha')
    if not isinstance(candidate, str) or len(candidate) != 40 or any(c not in '0123456789abcdef' for c in candidate):
        raise InvalidEvidence('invalid exact candidate sha')
    _required_digest(item.get('provider_state_sha256'), 'provider state digest')
    if item.get('scope') != 'browser_observed_media_plane_only':
        raise InvalidEvidence('unsupported evidence scope')
    if item.get('participant_role') not in ('owner', 'visitor'):
        raise InvalidEvidence('unknown role; never infer it from filename')
    _positive_sequence(item.get('session_sequence'), 'session sequence')
    _nonnegative(item.get('session_duration_millis'), 'session duration')
    if not isinstance(item.get('voice_attempts'), list) or not isinstance(item.get('media_events'), list):
        raise InvalidEvidence('missing voice/media arrays')
    if not isinstance(item.get('canonical_playback_proven'), bool) or not isinstance(item.get('av_sync_proven'), bool):
        raise InvalidEvidence('missing canonical proof booleans')
    if not isinstance(item.get('av_sync_samples'), list) or not isinstance(item.get('av_sync_diagnostics'), list):
        raise InvalidEvidence('missing A/V proof arrays')
    requests: set[int] = set()
    completed_requests: set[int] = set()
    for attempt in item['voice_attempts']:
        if not isinstance(attempt, dict) or attempt.get('status') not in ('completed', 'failed', 'cancelled', 'denied'):
            raise InvalidEvidence('invalid voice attempt')
        sequence = _positive_sequence(attempt.get('request_sequence'), 'voice request sequence')
        if sequence in requests:
            raise InvalidEvidence('duplicate voice request sequence')
        requests.add(sequence)
        if attempt['status'] == 'completed':
            completed_requests.add(sequence)
            _nonnegative(attempt.get('stt_millis'), 'STT time')
            _nonnegative(attempt.get('llm_millis'), 'LLM time')
            if not isinstance(attempt.get('canonical_playback_confirmed'), bool):
                raise InvalidEvidence('missing per-turn canonical playback flag')
    seen_events: set[tuple[int | None, str]] = set()
    started_audio_requests: set[int] = set()
    stopped_requests: set[int] = set()
    for event in item['media_events']:
        if not isinstance(event, dict) or not isinstance(event.get('kind'), str):
            raise InvalidEvidence('malformed media event')
        kind = event['kind']
        if kind not in SESSION_MEDIA_KINDS and kind not in VOICE_MEDIA_KINDS:
            raise InvalidEvidence('unknown canonical media event kind')
        _nonnegative(event.get('elapsed_millis'), 'media event elapsed time')
        sequence = event.get('request_sequence')
        if kind in SESSION_MEDIA_KINDS:
            # An avatar can render video BEFORE anyone has spoken.
            # Session lifecycle events must never fabricate a voice request.
            if sequence is not None:
                raise InvalidEvidence('session media event must not reference a voice request')
        else:
            sequence = _positive_sequence(sequence, 'media request sequence')
            if sequence not in requests:
                raise InvalidEvidence('media event references unknown voice request')
            if sequence not in completed_requests:
                raise InvalidEvidence('turn media event lacks a completed voice request')
        event_key = (sequence, kind)
        if kind != 'reconnect_restored' and event_key in seen_events:
            raise InvalidEvidence('duplicate canonical media event')
        seen_events.add(event_key)
        if kind == 'audio_started':
            started_audio_requests.add(sequence)
        elif kind == 'interruption_stopped':
            stopped_requests.add(sequence)
    # STOP can only reference audio from the SAME completed voice turn.
    # Use two-phase validation because canonical snapshots need not be ordered.
    if not stopped_requests.issubset(started_audio_requests):
        raise InvalidEvidence('STOP lacks matching started audio for the same voice request')
    return item, hashlib.sha256(raw).hexdigest()

def _millis(item: dict[str, Any], kind: str) -> float | None:
    values = [float(e['elapsed_millis']) for e in item['media_events'] if e.get('kind') == kind]
    return min(values) if values else None

def _quality(item: dict[str, Any]) -> dict[str, Any]:
    completed = [a for a in item['voice_attempts'] if a.get('status') == 'completed']
    return {
        'role': item['participant_role'],
        'completed_voice_turns': len(completed),
        'first_useful_video_millis': _millis(item, 'end_to_end_video_ready'),
        'first_browser_audio_millis': _millis(item, 'audio_started'),
        'observed_interrupt_stop_millis': _millis(item, 'interruption_stopped'),
        'reconnect_observed': _millis(item, 'reconnect_restored') is not None,
        'reported_canonical_playback_proven': item['canonical_playback_proven'],
        'reported_av_sync_proven': item['av_sync_proven'],
        'av_sync_samples': len(item['av_sync_samples']),
        'av_sync_diagnostics': sorted({
            str(d.get(k)) for d in item['av_sync_diagnostics'] if isinstance(d, dict)
            for k in ('audio_issue', 'video_issue') if isinstance(d.get(k), str)
        }),
        'stt_millis': min((float(a['stt_millis']) for a in completed), default=None),
        'llm_millis': min((float(a['llm_millis']) for a in completed), default=None),
    }

def build_handoff(paths: list[Path], speaker_quality: str = 'unreviewed') -> dict[str, Any]:
    if not paths:
        raise InvalidEvidence('at least one real exported snapshot is required')
    if speaker_quality not in ('unreviewed', 'rejected', 'accepted'):
        raise InvalidEvidence('invalid human speech-quality disposition')
    sessions, digests, sequences = [], set(), set()
    for path in paths:
        item, digest = _read(path)
        if digest in digests:
            raise InvalidEvidence('duplicate snapshot bytes')
        digests.add(digest)
        sequence = item['session_sequence']
        if sequence in sequences:
            raise InvalidEvidence('duplicate session sequence')
        sequences.add(sequence)
        sessions.append(item)
    candidate = sessions[0]['candidate_sha']
    state = sessions[0]['provider_state_sha256']
    if any(x['candidate_sha'] != candidate or x['provider_state_sha256'] != state for x in sessions):
        raise InvalidEvidence('cross-candidate or cross-provider-state evidence')
    by_role = {role: [_quality(x) for x in sessions if x['participant_role'] == role]
               for role in ('owner', 'visitor')}
    observed = {
        'owner_voice': any(x['completed_voice_turns'] for x in by_role['owner']),
        'visitor_voice': any(x['completed_voice_turns'] for x in by_role['visitor']),
        'owner_stop': any(x['observed_interrupt_stop_millis'] is not None for x in by_role['owner']),
        'owner_video': any(x['first_useful_video_millis'] is not None for x in by_role['owner']),
        'visitor_video': any(x['first_useful_video_millis'] is not None for x in by_role['visitor']),
        'owner_audio': any(x['first_browser_audio_millis'] is not None for x in by_role['owner']),
        'visitor_audio': any(x['first_browser_audio_millis'] is not None for x in by_role['visitor']),
    }
    # Browser observation is not authoritative completion or human identity proof.
    # One STOP observation cannot establish a p95 quality contract.
    gates = [{
        'id': key, 'requirement': description,
        'state': 'observed_not_exit_verified' if observed.get(key) else 'not_evidenced',
    } for key, description in GATES]
    if speaker_quality == 'rejected':
        for gate in gates:
            if gate['id'] == 'human_quality':
                gate['state'] = 'human_rejected'
    guidance = {
        'human_quality': 'Trial a rights-cleared natural Russian speech model; obtain actual owner and visitor human ratings.',
        'visitor_voice': 'Invite a distinct non-owner human; record a consented visitor-scoped real-provider voice session.',
        'canonical_playback': 'Capture exact-turn provider-confirmed full playout, not browser first-audio alone.',
        'av_sync': 'Resolve missing W3C estimatedPlayoutTimestamp or honestly retain strict RT0 A/V proof as unavailable.',
        'reconnect': 'Exercise a real connection loss and canonical recovery with measured reconnect latency.',
        'claim_correction': 'Correct a reviewed owner claim and prove the next real answer uses the new PersonaVersion.',
        'revocation_deny': 'Revoke access, attempt new protected action, and verify no downstream provider egress.',
        'billing': 'Capture real provider cost/usage including avatar and review total per duration; do not invent charges.',
        'golden': 'Run the bound private owner Golden suite on this exact candidate/provider state.',
        'privacy': 'Complete reviewed permissions, non-owner private-context and false-owner-opinion-attribution acceptance.',
        'distinct_visitor': 'Independently attest participant identity and distinguish the visitor from the owner.',
    }
    priority = ('human_quality', 'visitor_voice', 'canonical_playback', 'av_sync',
                'reconnect', 'claim_correction', 'revocation_deny', 'billing',
                'golden', 'privacy', 'distinct_visitor')
    open_states = {gate['id']: gate['state'] for gate in gates}
    next_actions = [
        {'gate': key, 'action': guidance[key]}
        for key in priority if open_states[key] != 'observed_not_exit_verified'
    ]
    return {
        'schema_version': OUTPUT_SCHEMA,
        'disposition': 'NOT_RELEASE_EVIDENCE',
        'release_ready_claimed': False,
        'candidate_sha': candidate,
        'provider_state_sha256': state,
        'snapshot_digests': sorted(digests),
        'snapshot_count': len(sessions),
        'speaker_quality_human_input': speaker_quality,
        'observations': by_role,
        'gates': gates,
        'next_actions': next_actions,
        'limitations': [
            'Role labels are not proof of distinct human visitor provenance.',
            'Browser media events are not provider-confirmed complete playback or A/V sync.',
            'One interruption sample does not establish p95 latency.',
            'No transcript, raw biometric, private Persona, or provider credentials are retained.',
            'Canonical release verdict comes only from vpr-rt0-exit-evidence.',
        ],
    }

def render_markdown(report: dict[str, Any]) -> str:
    lines = [
        '# RT0 live-session acceptance handoff (NON-PROMOTING)',
        '',
        f"Candidate: {report['candidate_sha']}",
        f"Provider state SHA-256: {report['provider_state_sha256']}",
        f"Snapshots: {report['snapshot_count']}",
        f"Human Russian TTS quality: **{report['speaker_quality_human_input']}**",
        '',
        '| Canonical RT0 gate | Current evidence |',
        '|---|---|',
    ]
    for gate in report['gates']:
        lines.append(f"| {gate['requirement']} | {gate['state']} |")
    lines.extend(['', '## Observed (not human/proven playback evidence)', ''])
    for role in ('owner', 'visitor'):
        for observation in report['observations'][role]:
            lines.append(
                f"- {role}: {observation['completed_voice_turns']} completed voice turns; "
                f"video {observation['first_useful_video_millis']} ms; "
                f"first observed audio {observation['first_browser_audio_millis']} ms; "
                f"observed STOP {observation['observed_interrupt_stop_millis']} ms; "
                f"canonical playback reported {observation['reported_canonical_playback_proven']}; "
                f"A/V reported {observation['reported_av_sync_proven']}; "
                f"A/V samples {observation['av_sync_samples']}"
            )
    lines.extend(['', '## Next work (not automatically performed)', ''])
    lines.extend(f"- {x['gate']}: {x['action']}" for x in report['next_actions'])
    lines.extend(['', '## Limitations', ''])
    lines.extend(f'- {v}' for v in report['limitations'])
    lines.append('')
    return '\n'.join(lines)

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('snapshots', nargs='+', type=Path, help='raw sanitized Owner Lab session JSON')
    parser.add_argument('--speech-quality', choices=('unreviewed', 'rejected', 'accepted'), default='unreviewed',
                        help='explicit HUMAN assessment, not inferred from timings')
    parser.add_argument('--format', choices=('markdown', 'json'), default='markdown')
    args = parser.parse_args()
    try:
        report = build_handoff(args.snapshots, args.speech_quality)
    except (InvalidEvidence, OSError) as exc:
        # Never echo raw file content, private prompts or untrusted file paths.
        parser.error(str(exc))
    print(json.dumps(report, ensure_ascii=False, indent=2) if args.format == 'json'
          else render_markdown(report))
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
