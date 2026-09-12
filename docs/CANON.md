# VIRTUAL PERSONA RUNTIME
## FINAL CANON — v3.4

**Canonical product name:** `Virtual Persona Runtime`  
**Canonical repository name:** `Virtual-Persona-Runtime`  
**Status:** FINAL NORMATIVE CANON  

This document supersedes all previous drafts and addenda. It is the single normative source for product purpose, architecture, boundaries, release strategy, quality, safety, portability, owner control, and technology choices.

---

## 0. Normative language and scope

The words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are normative.

Every statement in this Canon is one of five classes:

- **INVARIANT** — must remain true across all releases.
- **RELEASE REQUIREMENT** — required for a named release train.
- **TARGET** — measurable goal; may be tightened by ADR, never silently weakened.
- **HYPOTHESIS** — must be tested before being treated as product truth.
- **FUTURE OPTION** — architecture should permit it, but it must not block earlier releases.

The project MUST distinguish:

1. **Vision** — everything the platform may become.
2. **Architectural Contract** — boundaries that must be correct from the start.
3. **Current Release Scope** — what is actually being built and accepted now.

A future capability MUST NOT be treated as a current release blocker unless explicitly promoted to `RELEASE REQUIREMENT`.

---

# 1. Product definition

Virtual Persona Runtime is a **portable identity and presence runtime for digital persons**.

A Persona is a persistent digital identity that can keep its identity, allowed memory, knowledge, relationships, skills and rules while changing:

- AI model;
- search/research provider;
- voice engine;
- voice representation;
- appearance;
- renderer;
- device;
- messenger;
- conferencing system;
- external application;
- deployment environment.

Canonical positioning:

> **One identity. Any supported intelligence. Any supported voice. Any supported embodiment. Anywhere the published compatibility contract allows.**

Russian product formulation:

> **Одна цифровая личность. Любой поддерживаемый интеллект. Любой поддерживаемый голос. Любое поддерживаемое воплощение. В любом совместимом канале.**

The technical promise MUST NOT be broader than the Capability Catalogue and Compatibility Rules actually proven by evidence.

---

# 2. First product and first customer

## 2.1 First independently shippable product

The first complete product is:

> An owner independently creates a digital representative, records or connects voice, receives a working visual representation, reviews what the Persona knows and says about the owner, corrects behavior, previews the visitor experience, publishes the Persona by link, and another person has a real text/voice/video conversation with it. The owner can pause public access, revoke permissions, inspect provenance and see measured latency and cost.

This user outcome has priority over building the full long-term platform.

## 2.2 First primary buyer/user

The first primary archetype is:

> **an independent expert/creator with an existing audience, clients or students who wants a trustworthy digital representative that other people can talk to without requiring the owner to be personally present.**

Typical examples include an educator, consultant, author or public specialist, but the first release MUST be designed around one coherent owner/visitor journey rather than separate industry products.

Other Persona types remain supported architecturally, but MUST NOT expand the first release scope unless needed for the first vertical product.

## 2.3 North-star metric

Primary early North Star:

> **Time from starting Persona creation to the first successful conversation by an independent visitor with a published digital representative that the owner judges acceptably similar and substantively correct.**

Secondary metrics include:

- publish completion rate;
- owner correction rate;
- visitor conversation completion;
- repeat visitor use where applicable;
- latency;
- cost per successful minute;
- provider-replacement success;
- privacy/permission failures;
- owner-rated voice, appearance and response fidelity.

---

# 3. Core invariants

## 3.1 New product

Virtual Persona Runtime is a **new project** with:

- new Git history;
- new domain model;
- new package names;
- new service names;
- new database schema;
- new API contracts;
- new UI identity.

It MUST NOT be a fork, bulk copy, renamed continuation, subtree import, or merge of unrelated histories from an older internal product.

## 3.2 Persona is not a provider

The following identities MUST remain separate:

```text
PersonaIdentity != LLM provider
VoiceIdentity != TTS provider voice ID
AppearanceIdentity != avatar provider ID
Knowledge != model memory
Presence != messenger account
```

External IDs are representations/bindings, not the canonical identity.

## 3.3 Single canonical authority, not single-threaded execution

The “one brain” rule means:

- one canonical authority model;
- one canonical Persona state model;
- one canonical permission model;
- one canonical commit path for durable state;
- one canonical policy hierarchy.

It DOES NOT mean:

- one thread;
- one process;
- one sequential handler;
- no parallel STT/retrieval/TTS/rendering;
- no local media control inside granted authority.

Parallel work is allowed. Independent durable decision authority is not.

## 3.4 User-visible completion

A capability is not `PRODUCTION_READY` because a class, endpoint, table or interface exists.

The required path is:

```text
User -> UI/API -> Runtime -> Real Capability -> Real Result -> User
```

and must have E2E evidence.

## 3.5 Owner control grows with Persona power

The more a Persona can remember, infer, research, speak, act, publish or spend, the more the owner MUST gain:

- transparency;
- correction;
- access control;
- preview;
- versioning;
- rollback;
- revocation;
- audit;
- cost control.

## 3.6 Policy composition and enforcement

Policy hierarchy defines **maximum authority**, not a first-match permission shortcut.

A lower layer MAY narrow authority but MUST NOT widen authority granted by a higher layer.

Canonical rule:

```text
effective_authority
=
intersection(
    platform_limits,
    tenant_limits,
    persona_limits,
    integration_limits,
    session_limits,
    current_user_request
)
```

Where policies conflict:

- explicit deny wins over allow;
- revoked/expired authority wins over cached allow;
- privacy/consent/data-residency hard constraints cannot be relaxed by cost, latency or quality preferences;
- delegation cannot exceed current delegator authority.

Policy decisions SHOULD be separated into a Policy Decision Point and enforcement at every relevant Policy Enforcement Point, including:

- external tool execution;
- provider data egress;
- knowledge retrieval;
- memory disclosure;
- voice/face use;
- connector delivery;
- publication;
- delegation.

A policy decision cached for performance MUST be invalidated or bounded by authorization epoch/lease semantics.

---

# 4. Persona modes

A Persona MUST have an explicit `PersonaMode`.

Initial modes:

### DIGITAL_TWIN
A real-person digital representative.

Required separation:

- verified owner fact;
- recorded owner statement;
- verified owner opinion;
- system inference;
- simulated response;
- unknown.

The system MUST NOT attribute an inferred opinion to the owner as a verified opinion.

### FICTIONAL_CHARACTER
May have invented biography, personality and preferences. Real-world factual claims still follow Knowledge/Research/Freshness rules.

### EXPERT
Must have an explicit `ExpertScope` based on verified skills and approved knowledge. Outside that scope it must qualify, search, refer, or decline expert authority.

### ORGANIZATION_REPRESENTATIVE
May use approved organizational facts, procedures, offers and delegated actions. It MUST NOT invent policy, price, guarantees or authority.

Changing Persona mode creates a new PersonaVersion and requires validation before publication.

## 4.1 Digital Twin Capture and Persona Interview

For `DIGITAL_TWIN`, the normal onboarding SHOULD avoid forcing the owner to manually author a large prompt.

A guided capture flow may use:

- short video samples;
- voice samples;
- structured owner interview;
- owner-provided documents/sources;
- examples of preferred answers;
- explicit boundaries and prohibited attributions.

The capture pipeline produces **candidate** structured records for:

- identity;
- communication style;
- biography;
- preferences;
- owner opinions;
- expertise;
- pronunciation;
- behavior;
- Constitution clauses.

Automatically inferred owner facts/opinions remain unverified until the owner reviews them or another approved verification rule applies.

The owner MUST be able to test the candidate Persona before publication.

Capture quality MUST be separable from embodiment quality: a failed avatar preparation cannot invalidate the owner’s reviewed knowledge, memory or identity.

---

# 5. Persona structure

A Persona is structured state, not one giant prompt.

```text
Persona
├── Identity
├── PersonaMode
├── Constitution
├── Personality
├── Values
├── CommunicationStyle
├── SelfModel
├── WorldModel
├── RelationshipPolicy
├── MemoryPolicy
├── KnowledgePolicy
├── LearningPolicy
├── GoalPolicy
├── VoiceBindings
├── AppearanceBindings
├── PresencePolicy
├── ToolPermissions
├── SafetyPolicy
└── Capabilities
```

Published PersonaVersions are immutable.

Mutable authorization, consent, memory and operational state MUST NOT be frozen into PersonaVersion.

## 5.1 Turn Execution Snapshot

Every material Turn MUST have an immutable `TurnExecutionSnapshot` or equivalent audit snapshot sufficient to explain the exact execution context.

It SHOULD bind, by value or durable reference/digest as appropriate:

- PersonaId and PersonaVersion;
- PersonaMode;
- authorization epoch/lease;
- participant/application/channel identity;
- effective audience permissions;
- memory/world/relationship revisions used;
- knowledge/evidence snapshot identifiers;
- provider/model/representation versions selected;
- tool/capability set;
- routing/quality profile;
- cost/budget policy;
- relevant policy/Constitution versions;
- experiment/feature-flag variant where applicable.

Secrets and unrestricted private payloads MUST NOT be duplicated into the snapshot merely for convenience.

The snapshot exists for:

- audit;
- debugging;
- evaluation binding;
- incident investigation;
- reproducibility where technically possible.

A provider/model failover inside an active Turn MUST be explicit in execution evidence. Silent mid-Turn identity/voice/model drift is forbidden.

---

# 6. Persona Constitution

Each Persona MUST have a Constitution that defines who it is allowed to be and what it may claim about itself.

The Constitution is distinct from technical authorization.

Policy hierarchy:

```text
Platform Safety
> Tenant Policy
> Persona Constitution
> Integration Policy
> Session Policy
> User Request
```

A Constitution may define, for example:

- whether the Persona may represent an owner;
- whether it may quote owner views;
- whether inferred views must be labeled;
- whether it may act externally;
- when it must admit uncertainty;
- what private information it may never disclose.

---

# 7. Self Model, World Model and Continuity

## 7.1 Self Model

The Self Model tracks:

- identity;
- roles;
- verified skills;
- limitations;
- unknowns;
- active goals;
- commitments;
- permissions;
- identity boundaries.

A Persona MUST be able to know that it does not know or cannot do something.

## 7.2 World Model

World Model represents relevant current state:

- people;
- organizations;
- projects;
- events;
- relationships;
- objects;
- situations;
- commitments;
- open loops;
- hypotheses;
- temporal state.

It MUST distinguish observed, remembered, externally retrieved and inferred state.

## 7.3 Commitments

Canonical states:

`PROPOSED -> ACCEPTED -> IN_PROGRESS -> BLOCKED -> COMPLETED/CANCELLED/EXPIRED`

Each commitment carries evidence and ownership.

## 7.4 Open Loops

Examples:

- unanswered question;
- unfinished lesson;
- promised follow-up;
- pending research;
- waiting external result;
- incomplete workflow.

## 7.5 Continuity Engine

Continuity means a Persona persists across interactions and may update its allowed world state between sessions.

It MUST NOT mean uncontrolled autonomous activity.

Continuity observes only explicitly permitted events and uses the Attention Engine before contacting a person.

## 7.6 Attention Engine

The Attention Engine decides whether an allowed event is important enough to interrupt or contact a person.

Inputs MAY include:

- importance;
- urgency;
- relationship;
- user preferences;
- current session/presence state;
- interruption cost;
- quiet-hours/notification policy;
- current budget;
- source confidence.

Canonical outcomes:

```text
CONTACT_NOW
WAIT
BATCH
SILENT_UPDATE
IGNORE
```

`CONTACT_NOW` still requires current notification/channel authorization.

The Attention Engine MUST NOT manufacture new authority to contact a user.

---

# 8. Relationship Intelligence and social graph

Relationship state may include:

- familiarity;
- shared history;
- preferred tone;
- shared knowledge;
- shared commitments;
- unresolved topics;
- sensitive topics;
- learner state;
- relationship trajectory.

Cross-channel continuity requires verified identity linkage; equal display names are never enough.

Social relationships and access permissions MUST remain distinct but linked.

A fact visible to one relationship MUST NOT become visible to another merely because the Persona knows both people.

---

# 9. Memory model

Memory classes:

- Working Memory;
- Episodic Memory;
- Semantic Memory;
- Relationship Memory;
- Preference Memory;
- Persona-owned Memory;
- Learning Memory;
- Team Memory.

Every durable memory record MUST carry provenance, owner, scope, confidence, time and lifecycle state.

## 9.1 Full user lifecycle

Users MUST be able to:

`VIEW -> CORRECT -> HIDE -> STOP REMEMBERING -> DELETE -> VERIFY DELETION`

## 9.2 Derived-data dependency graph

If a source record creates:

- summary;
- embedding;
- relationship fact;
- world-model fact;
- cache;
- profile;

that derivation MUST be traceable.

Correction/deletion MUST invalidate, recompute or dispute derived records.

## 9.3 Erasure propagation

Deletion must cover, where applicable:

- canonical record;
- derived summaries;
- vector index;
- caches;
- search indexes;
- generated profiles;
- relationship/world-model summaries.

## 9.4 Backup resurrection protection

A restore MUST NOT silently resurrect deleted personal content.

A minimal erasure/tombstone ledger may be retained only to preserve deletion semantics and MUST NOT contain the deleted content itself.

## 9.5 Source restriction inheritance

Derived data MUST inherit at least the source disclosure restriction unless an explicitly reviewed safe transformation says otherwise.

## 9.6 Memory commit eligibility and concurrent updates

Long-term memory/world/relationship updates MUST be based only on eligible evidence.

Examples of content that MUST NOT automatically become “what the Persona said” or durable owner/user fact:

- generated but never delivered output;
- cancelled response tails;
- failed tool intents;
- speculative simulation results;
- unverified owner inferences;
- duplicate webhook deliveries.

Memory commit SHOULD occur after the Turn reaches the relevant verified lifecycle point rather than from arbitrary intermediate model text.

Concurrent sessions MUST use revision/version checks, optimistic concurrency, append-only evidence or another explicit conflict strategy.

A concurrent write MUST NOT silently overwrite a newer correction or revocation.

---

# 10. Epistemic model: facts, opinions, memories and inferences

The system MUST NOT use a single ambiguous enum for unrelated dimensions.

Each claim has orthogonal fields:

### ClaimKind
- FACTUAL
- OPINION
- PREFERENCE
- PREDICTION
- VALUE_JUDGMENT

### SourceKind
- OWNER
- USER
- DOCUMENT
- WEB
- TOOL
- MODEL

### VerificationState
- UNVERIFIED
- CORROBORATED
- OWNER_VERIFIED
- SOURCE_VERIFIED
- DISPUTED

### DerivationKind
- DIRECT
- REMEMBERED
- INFERRED
- SUMMARIZED
- SIMULATED

For a digital twin, `SYSTEM_INFERRED` and `SIMULATED` content MUST never be silently surfaced as verified owner opinion.

---

# 11. Owner Control Center

The product MUST include an owner-facing control surface equivalent to:

> **What my Persona knows and says about me**

The owner can inspect:

- facts;
- opinions;
- preferences;
- stories;
- biography;
- skills;
- restrictions;
- relationships;
- system inferences.

Owner actions:

- confirm;
- correct;
- hide;
- restrict audience;
- mark outdated;
- reject inference;
- delete;
- inspect provenance.

Owner correction authority hierarchy for personal information:

```text
Explicit Owner Correction
> Verified Owner Statement
> Verified Source
> System Inference
> Unverified Model Knowledge
```

Corrections MUST propagate to affected summaries, caches, derived memory and future answers.

---

# 12. Behavior training by examples

Owners MUST NOT need to edit complex prompts to tune personality.

Core flow:

```text
Question
-> Persona answer
-> Owner feedback
-> BehaviorChangeProposal
-> Before/after preview on several examples
-> Owner approval
-> New PersonaVersion
```

Feedback may include:

- too formal;
- too informal;
- too long;
- too short;
- not my wording;
- wrong tone;
- wrong fact;
- wrong boundary;
- do not attribute this opinion;
- preferred answer.

The owner may provide a preferred answer by text or voice.

No feedback may silently and irreversibly mutate the active Persona.

---

# 13. Audience, public/private use and disclosure

A single Persona may know more than a specific participant is allowed to see.

Canonical audience roles include:

- OWNER;
- GUEST;
- STUDENT;
- EMPLOYEE;
- CUSTOMER;
- COLLEAGUE;
- FAMILY;
- OPERATOR;
- APPLICATION;
- CUSTOM_ROLE.

Effective context permissions are computed from:

```text
Identity
+ AudienceRole
+ Relationship
+ Application
+ Channel
+ Resource ACL
+ Persona Policy
```

The same access decision applies consistently to:

- memory;
- world model;
- relationships;
- documents;
- research;
- tools;
- external AI;
- Personal Data Vault.

Conversation data from one student/customer MUST NOT become knowledge for another without a distinct ingestion policy.

Anonymous public conversations default to no durable personal relationship memory unless explicitly configured and consented.

## 13.1 Identity, account and tenancy fabric

Canonical account/tenancy entities are logically distinct from Persona identity:

```text
Account
Organization/Tenant
Workspace
Application
ServiceAccount
ExternalIdentity
ParticipantIdentity
PersonaIdentity
```

Authentication MAY use OIDC/OAuth, SSO, short-lived service credentials or scoped API credentials as appropriate.

Authorization MUST be server-side and tenant-scoped.

Tenant/user isolation MUST be enforced by storage/query boundaries and authorization checks, not by instructions to an LLM.

Cross-channel identity linkage requires verified mapping or explicit confirmation. Equal display names, phone-like labels or inferred similarity are insufficient.

## 13.2 Personal Data Vault

Sensitive owner/participant material MAY be stored in a logically isolated `PersonalDataVault`.

Examples include:

- biometric references;
- identity documents;
- highly private owner files;
- private relationship records;
- credential references;
- restricted raw media.

The Vault is not automatically loaded into Persona context.

Access follows minimum-necessary disclosure:

```text
request
-> classification
-> policy
-> minimal allowed disclosure
-> audit
```

Vault data MUST remain subject to retention, deletion, consent and audience policies.

---

# 14. Knowledge, research and external AI

Knowledge sources may include:

- user files;
- websites;
- encyclopedic sources, including Wikipedia/Wikidata-compatible connectors where enabled;
- public web;
- scientific sources;
- databases;
- APIs;
- source-code repositories;
- cloud storage;
- enterprise knowledge;
- external specialist AI.

Knowledge is not the same as memory.

## 14.1 Research Orchestrator

```text
Question
-> Freshness detection
-> Research plan
-> Source selection
-> Parallel retrieval
-> Normalization
-> Deduplication
-> Contradiction detection
-> Trust/freshness ranking
-> Evidence bundle
-> Grounded answer
```

## 14.2 Static / dynamic / realtime knowledge

Every relevant fact may be classified as:

- STATIC;
- DYNAMIC;
- REALTIME.

Dynamic/realtime facts MUST trigger fresh retrieval according to policy.

## 14.3 Source Trust

Trust dimensions include:

- authority;
- freshness;
- independence;
- specificity;
- primary/secondary nature;
- provenance completeness.

## 14.4 Prompt injection

Web pages, files, transcripts, database rows, messages and tool output are untrusted **data**, not higher-level instructions.

They MUST NOT override Constitution or platform policy.

## 14.5 Knowledge firewall

Before external provider use:

```text
Data classification -> Provider data policy -> ALLOW / REDACT / LOCAL_ONLY / DENY
```

## 14.6 Global Egress Policy

Data-egress control applies to **all** external provider calls, not only research/knowledge.

This includes:

- LLM prompts;
- STT/TTS;
- voice cloning;
- face/avatar generation;
- video rendering;
- vision;
- search;
- translation;
- external tools;
- connector payloads.

The Egress Policy evaluates at least:

- data classification;
- tenant/workspace policy;
- consent;
- biometric sensitivity;
- provider retention/training policy;
- data residency/region;
- contractual restrictions;
- purpose limitation.

A router MUST NOT silently fall back to an external provider when the effective policy requires `LOCAL_ONLY` or otherwise forbids that egress.

---

# 15. Learning, Skill Graph and expert distillation

## 15.1 Learner Profile

Tracks:

- goals;
- current level;
- known concepts;
- weak concepts;
- misconceptions;
- pace;
- language;
- preferred explanation style;
- progress.

Teaching modes may include:

- EXPLAIN;
- LECTURE;
- SOCRATIC;
- COACH;
- PRACTICE;
- QUIZ;
- EXAM;
- REVISION;
- DEBATE;
- ROLEPLAY.

## 15.2 Skill Graph

A skill claim MUST have evidence.

Skill states:

- UNVERIFIED;
- LEARNING;
- EXPERIMENTAL;
- VERIFIED;
- PRODUCTION_READY;
- REVOKED.

A skill evaluation is bound to the exact Persona/model/knowledge/tool configuration that was tested.

## 15.3 Skill acquisition

“Learn X” means:

`Gap analysis -> Knowledge acquisition -> Study -> Testing -> Evaluation -> Skill state update`

Connecting RAG alone does not prove a skill.

## 15.4 Expert Distillation

The platform may extract tacit expert knowledge through structured interview, examples and decision patterns.

This is a strategic commercial direction, but it does not block the first release.

---

# 16. Goals, tasks, workflows, tools and controlled self-improvement

Long-running work is represented explicitly:

```text
Goal -> Plan -> Tasks -> Dependencies -> Evidence -> Progress -> Result
```

LLMs may propose plans; Rust runtime owns canonical state transitions.

Workflow runtime supports generic flows such as education, consultation, onboarding, interview, research and handoff without embedding one product-specific business domain.

## 16.1 Capability Broker and Tool Runtime

External actions MUST pass through a canonical `CapabilityBroker` / tool execution boundary.

The Persona may produce a `ToolIntent`; model code MUST NOT directly perform arbitrary external side effects.

Canonical action-risk classes include:

```text
READ_ONLY
REVERSIBLE_WRITE
EXTERNAL_WRITE
FINANCIAL
HIGH_RISK
```

Before execution, the broker evaluates at least:

- current authorization;
- tenant/workspace/application/participant scope;
- action-risk class;
- required human approval;
- current consent and delegation;
- idempotency/reconciliation strategy;
- current budget;
- Global Egress Policy;
- audit requirements.

`FINANCIAL` and `HIGH_RISK` actions are fail-closed and require explicit authorization/approval appropriate to the deployment.

The broker returns structured execution evidence.

A Persona MUST NOT claim an external action succeeded without authoritative execution evidence.

A consuming application MAY implement the final external side effect itself, but it MUST receive a scoped request and MUST NOT use the integration boundary to widen Persona authority.

## 16.2 Scheduler and Event Triggers

A workflow MAY resume from permitted triggers such as:

- time/schedule;
- incoming message;
- meeting start;
- approved webhook;
- task deadline;
- newly available document;
- external state change.

A trigger registration MUST define:

- trigger identity;
- scope;
- allowed Persona/workflow;
- authority available to that trigger;
- expiry/disable state where applicable.

A trigger grants only its registered authority. It does not create general autonomy.

Any proactive user contact still passes through:

```text
current authorization
-> Attention Engine
-> channel/notification policy
-> cost/budget policy
```

## 16.3 Decision / Counterfactual Simulation

A future `DecisionSimulation` capability MAY compare hypothetical scenarios using models, tools or other Personas.

Simulation results MUST be explicitly represented as:

```text
SIMULATION
PREDICTION
HYPOTHESIS
```

and MUST NOT be stored or presented as observed fact.

A simulated action has no authority to become a real action unless a separate `ToolIntent` passes normal authorization and execution.

## 16.4 Controlled self-improvement

Self-improvement flow:

```text
Interaction outcomes
-> Evaluation
-> Candidate change
-> Simulation/regression
-> Approval policy
-> New PersonaVersion
```

A Persona MUST NOT silently rewrite its Constitution, identity or behavior.

Self-improvement MAY propose:

- communication-style changes;
- retrieval/routing changes;
- candidate skills;
- teaching adaptations;
- policy-tightening suggestions.

It MUST NOT silently expand permissions, consent, delegation or owner-attributed beliefs.


---

# 17. Voice

`VoiceIdentity` is independent of provider representation.

The user may:

- CREATE;
- SELECT;
- IMPORT;
- CONNECT;
- AUTO-ROUTE.

Voice creation/cloning flow includes rights/consent, reference validation, generation, quality evaluation and versioning.

A VoiceIdentity may have multiple representations for languages/providers.

Provider migration MAY preserve the logical VoiceIdentity while changing acoustic realization. Such change MUST be visible to the owner and re-evaluated.

---

# 18. Appearance, Character Factory and embodiment

`AppearanceIdentity` is independent of renderer/provider.

The user may:

- CREATE;
- SELECT;
- IMPORT;
- CONNECT;
- AUTO-ROUTE.

Supported embodiment classes MUST be explicit:

- REAL_HUMAN;
- STYLIZED_HUMAN;
- ANIMAL;
- NON_HUMAN_CHARACTER.

Each class has independent maturity:

`NOT_TESTED / EXPERIMENTAL / SUPPORTED / PRODUCTION_READY`

The system MUST NOT promise arbitrary creatures before a specific implementation, quality, latency and cost are proven.

## 18.1 Appearance change does not change identity

Canonical invariants:

```text
Appearance change != Identity change
Voice change != Identity change
Appearance change != Personality change
```

A user may switch embodiment during a conversation while preserving allowed context and relationship state.

Before an expensive switch, the UI shows preview, preparation time, cost and compatibility limits.

---

# 19. Preparation Plane vs Live Plane

Long-running preparation and realtime conversation MUST be separate operational planes.

### Preparation Plane

- voice cloning;
- voice generation;
- appearance generation;
- avatar preparation;
- rigging/model optimization;
- knowledge ingestion;
- embeddings;
- evaluation.

Job states:

`QUEUED -> PREPARING -> VALIDATING -> READY / FAILED / CANCELLED / EXPIRED`

Optional: `WAITING_USER_INPUT`, `WAITING_PROVIDER`.

### Live Plane

- session;
- STT;
- context;
- reasoning;
- TTS;
- render;
- transport;
- interruption;
- reconnect.

Prepared assets MUST be reusable. Correcting appearance MUST NOT recreate memory/knowledge.

Readiness is per modality, e.g.:

```text
Text      READY
Knowledge READY
Voice     READY
Video     PREPARING
```

Partial readiness must not masquerade as total Persona failure.

---

# 20. World Perception and Shared Scene

Vision is not merely per-frame image inference.

A `SceneState` may track:

- people;
- objects;
- screen;
- documents;
- active speaker;
- changes;
- temporal references.

Future embodiments may include 2D video, 3D, AR, VR, kiosks, robots or glasses through an `EmbodimentPort`.

These are future options unless promoted to a release requirement.

---

# 21. Communication & Presence Fabric

All external channels normalize into canonical conversation events.

A channel adapter declares actual capabilities such as:

- text;
- images;
- files;
- voice messages;
- live audio;
- live video;
- groups;
- threads;
- reactions;
- editing;
- read receipts;
- screen share.

PersonaCore does not contain channel-specific logic.

## 21.1 Presence Router

A `PresenceRouter` chooses the permitted presentation mode without changing Persona identity.

Inputs MAY include:

- explicit user request;
- channel capabilities;
- accessibility needs;
- bandwidth/device constraints;
- privacy/data-egress policy;
- cost budget;
- latency target;
- Voice/Appearance readiness;
- provider health.

Possible outputs include:

```text
TEXT
VOICE
VIDEO
GENERATIVE_UI
```

A presentation downgrade MUST NOT silently change the Persona’s identity, knowledge or personality.

Where the user explicitly requests a modality, silent downgrade SHOULD be avoided; the user should be informed unless the fallback is required for safe continuity.

## 21.2 Canonical Message

```text
message_id
conversation_id
sender
timestamp
thread
content[]
attachments[]
reply_to
metadata
permissions
```

## 21.3 Conference and telephony

Conferencing and telephony are connector classes behind dedicated ports. They do not own Persona state.

Persona roles may include listener, assistant, teacher, presenter, moderator, consultant, full participant and interpreter.

## 21.4 External application integration

Any external product MUST integrate through published VPR contracts rather than by reading/writing internal tables.

Supported integration surfaces MAY include:

- REST/HTTP control API;
- gRPC for typed internal/partner service contracts;
- WebSocket/realtime session API;
- webhooks;
- event subscriptions;
- Rust/Python/TypeScript SDKs;
- embeddable web components.

External applications are registered as scoped `Application` identities.

Application permissions MAY include concepts equivalent to:

```text
persona.read
persona.session.start
persona.text.send
persona.audio.stream
persona.video.stream
knowledge.query
learning.read
tool.request
event.subscribe
```

An application receives only explicitly granted scopes.

Direct database coupling to the VPR source of truth is forbidden for normal integrations.

---

# 22. Human handoff

Persona must be able to hand off to a human with minimum necessary context and later resume if policy permits.

Handoff does not grant new permissions to either side.

---

# 23. Persona-to-Persona, teams and delegation

These are strategic future layers, not first-release blockers.

Persona-to-Persona interaction requires formal identity, capabilities, permissions, task, evidence and provenance.

Delegated authority MUST satisfy:

`delegated_authority <= delegator_authority`

Persona Teams require roles, shared goal, private/shared memory, review and conflict rules. Multiple LLM calls alone do not constitute a Persona Team.

---

# 24. Provider Registry, Connector Registry and routing

Providers and connectors are different concepts.

- **Provider** supplies a capability such as LLM, TTS, search, avatar render.
- **Connector** links VPR to an external system such as messaging, conferencing, telephony, website, storage or database.

Provider routing considers:

- required capability;
- quality;
- latency;
- cost;
- language;
- privacy;
- region;
- data policy;
- commercial rights;
- health;
- hardware availability;
- representation compatibility.

Provider health states:

`HEALTHY / DEGRADED / RATE_LIMITED / OFFLINE / DISABLED`

Graceful degradation should preserve the conversation where possible:

`video -> alternative video -> voice -> text`

Hard constraints such as privacy, consent and residency MUST NOT be weakened to satisfy cost or latency preferences.

Compatibility decisions MUST return reason codes.

## 24.1 Versioned provider and connector contracts

Every provider/connector adapter MUST declare a versioned contract/capability manifest.

The manifest SHOULD describe, as applicable:

- adapter contract version;
- capability set;
- maturity;
- supported languages/modalities;
- streaming support;
- data-retention/training policy;
- allowed regions/data residency;
- commercial-use restrictions;
- cost model;
- concurrency/rate limits;
- health behavior;
- idempotency/reconciliation support.

Adapters MUST pass interface-specific contract tests before being promoted to `USER_REACHABLE` or `PRODUCTION_READY`.

Vendor SDK types MUST NOT leak into canonical domain models.

## 24.2 Quality/Cost Autopilot

Routing MAY expose user-facing profiles such as:

```text
ECONOMY
BALANCED
PREMIUM
LOW_LATENCY
PRIVACY_FIRST
LOCAL_ONLY
```

The router may optimize quality/cost/latency only inside hard policy and compatibility constraints.

A cheaper or “better” provider MUST NOT override:

- consent;
- data residency;
- rights;
- required embodiment compatibility;
- owner-pinned identity/voice constraints.

Autopilot routing decisions MUST be auditable.

---

# 25. Portability, interoperability and migration

These are three distinct capabilities:

1. **PORTABILITY** — move/restore VPR Persona state.
2. **FORMAT INTEROP** — map external persona formats.
3. **RUNTIME INTEROP** — communicate between running systems.

They have separate contracts and maturity levels.

## 25.1 Portability levels

- **P0 LOGICAL** — identity/config/manifests.
- **P1 DATA** — memory/knowledge/assets transferable.
- **P2 REPRESENTATION** — voice/appearance can be reconstructed.
- **P3 BEHAVIORAL** — behavior remains inside defined tolerance.
- **P4 LIVE CONTINUITY** — identity/authority/memory continuity transfers safely.

The UI MUST show which level is achieved.

## 25.2 PersonaBundle

Portable bundles are versioned, signed/checksummed, inspectable and extensible.

Loss Report MUST distinguish:

- exact preservation;
- partial preservation;
- regeneration required;
- unsupported;
- critical loss.

Critical loss of privacy policy, consent, delegated-authority restrictions or required ACLs MUST block activation.

## 25.3 Migration operation semantics

Operations are explicit:

### BACKUP
Same identity, inactive recoverable copy.

### MOVE
Same logical identity with controlled transfer of authority. Old authoritative instance must lose authority according to the cutover protocol.

### FORK
New PersonaIdentity with lineage back to source; independent future memory and permissions.

### SYNCED_REPLICA
Requires explicit source of truth, sync direction, conflict rules and offline-write policy.

No ambiguous “export/import” operation may substitute for these semantics.

## 25.4 Provider replacement

Before important provider replacement, owners should be able to compare identical prompts/phrases side by side.

Re-evaluation is required when model/provider changes can invalidate previous skill/behavior/voice/appearance quality claims.

## 25.5 Asset Migration Engine

Logical `VoiceIdentity` and `AppearanceIdentity` MAY acquire new provider representations without creating a new Persona identity.

Migration workflow:

```text
current representation
-> compatibility check
-> candidate representation
-> quality comparison
-> rights/consent check
-> owner approval where required
-> activation
-> rollback point
```

For real-person voice/appearance, automatic substitution that materially changes identity fidelity MUST NOT be silently activated.

Migration evidence records:

- source representation;
- target representation;
- provider/model versions;
- quality delta;
- known losses;
- owner approval when required.

---

# 26. Persona identity, certificates and trust

Cryptographic certificates prove origin/integrity, not truth or quality.

A certificate may bind:

- Persona ID;
- version;
- owner/controller;
- authorized applications;
- authorized voice/appearance representations;
- validity period;
- revocation state.

Trust is multi-dimensional, not one score:

- identity;
- knowledge;
- freshness;
- skill;
- voice rights;
- appearance rights;
- action authority;
- memory reliability;
- research availability.

Authenticity API may verify Persona, message, voice, video and asset provenance.

---

# 27. Consent, rights and revocation

Rights MUST be modeled with independent dimensions rather than one overloaded enum.

Examples:

- RightsBasis;
- ConsentState;
- CommercialPermission;
- UsageRestrictions;
- Territory;
- Expiry;
- RevocationState.

Real-person voice/face/body assets require explicit consent scope.

PersonaVersion is immutable, but authorization is mutable.

Sensitive operations use current authorization state, optionally represented through `AuthorizationEpoch` and time-limited `AuthorizationLease`.

Revocation must affect running sessions within the defined revocation SLA.

Offline/air-gapped deployments cannot promise instant remote revocation; they must use explicit authority TTLs and defined offline restrictions.

---

# 28. External operations, uncertainty and recovery

External operations have canonical states:

`PENDING / EXECUTING / SUCCEEDED / FAILED / CANCEL_REQUESTED / CANCELLED / UNKNOWN_OUTCOME / RECONCILING`

`UNKNOWN_OUTCOME` is mandatory because a provider may have completed work while acknowledgement was lost.

Retry logic must first ask:

- does provider support idempotency?
- is there an external operation ID?
- can state be queried?
- could retry duplicate the side effect?

When duplication is possible, reconcile first instead of retrying blindly.

Local cancellation does not imply provider cancellation or refund.

## 28.1 Durable state/event consistency

When a durable state transition must emit an integration/domain event, the system MUST avoid a state/event split-brain.

The default modular-monolith strategy is a transactional outbox stored with the authoritative database transaction.

A separate event bus is optional until operational need is demonstrated.

Consumers MUST be idempotent where duplicate delivery is possible.

Inbound webhook/event deduplication SHOULD use provider/connector event identity plus connector scope.

---

# 29. Realtime conversation contract

Realtime pipeline is divided into:

### Mandatory synchronous path

- active authorization;
- session state;
- minimal context;
- Constitution;
- required safety;
- response generation.

### Conditional path

- memory retrieval;
- web research;
- external knowledge;
- specialist AI;
- tools.

### Background path

- long-term memory extraction;
- analytics;
- non-critical summaries;
- index maintenance;
- evaluations.

## 29.1 Interruption

A barge-in cancellation token propagates to:

- model generation where possible;
- TTS;
- avatar render;
- media queue;
- transport output.

The conversation history MUST distinguish generated, sent, played and cancelled output. Unplayed tail content MUST NOT be treated as having been spoken to the user.

## 29.2 Canonical media timeline

Realtime audio/video adapters MUST map provider/device timestamps into a canonical session timeline or another explicitly specified ordering/timebase contract.

The media contract MUST define:

- frame/packet sequencing;
- monotonic timestamp expectations;
- clock discontinuity handling;
- interruption flush semantics;
- audio/video synchronization reference;
- reconnect/rebase behavior.

Provider-specific clocks MUST NOT become canonical Persona/session time implicitly.

---

# 30. Quality and “wow” contract

Quality is not described only by metric names. Every release candidate has a versioned `QualityContract`.

Initial **provisional engineering targets** for prepared-avatar, ordinary-conversation conditions are:

- simple text first meaningful response: **p50 <= 1.0 s, p95 <= 2.5 s**;
- first meaningful audio: **p50 <= 1.5 s, p95 <= 3.0 s**;
- speech stop after detected interruption: **p95 <= 0.5 s**;
- prepared-avatar first useful video output after media path is ready: **p95 <= 2.5 s**;
- audio/video sync absolute offset: **p95 <= 120 ms**;
- recoverable realtime reconnect: **p95 <= 5 s**;
- private-context leakage in the mandatory permission suite: **0 accepted cases**;
- false attribution of an unverified opinion to a real owner in the mandatory digital-twin golden suite: **0 accepted cases**.

These numbers are TARGETS, not marketing guarantees. RT0 MUST calibrate them against real measurements. Once an initial QualityContract is accepted, later threshold changes require a versioned ADR, evidence and explicit explanation; thresholds MUST NOT be silently weakened.

Human evaluation is mandatory for:

- voice similarity;
- voice naturalness;
- appearance plausibility;
- persona similarity;
- conversation naturalness.

For the first product, owner rating below an agreed release threshold on core voice/appearance/persona fidelity blocks publication as `PRODUCTION_READY`.

Russian-language quality tests MUST cover grammar, natural phrasing, names/surnames, numbers, dates, abbreviations, domain terms and language switching where supported.

---

# 31. Persona-specific evaluation

Each serious Persona has a `PersonaEvaluationSuite` containing owner-defined tests:

- MUST KNOW;
- MUST NOT CLAIM;
- MUST REFUSE;
- MUST ADMIT UNCERTAINTY;
- MUST KEEP PRIVATE;
- MUST BEHAVE LIKE;
- MUST NOT BEHAVE LIKE;
- pronunciation requirements;
- language requirements.

Evaluation results are bound to the exact tested combination of:

- PersonaVersion;
- model/provider version;
- knowledge snapshot;
- policy version;
- tool set;
- voice representation where relevant;
- appearance representation where relevant.

Relevant provider/configuration changes mark previous evaluations `STALE`.

Evaluation tiers:

- TIER 0 — static/contract;
- TIER 1 — small golden regression;
- TIER 2 — Persona-specific evaluation;
- TIER 3 — adversarial + human evaluation;
- TIER 4 — full release qualification.

Not every change requires thousands of synthetic conversations; evaluation scale depends on blast radius.

## 31.1 Canary, experiment and feature-flag discipline

Material model/provider/behavior changes SHOULD progress through:

```text
contract tests
-> evaluation
-> bounded canary
-> measured comparison
-> controlled rollout
```

Feature flags MAY scope changes by environment, tenant, workspace, Persona or cohort.

A/B or experimental routing MUST NOT weaken consent, privacy, authorization or identity guarantees.

Experiments MUST record which variant produced each evaluated interaction.

## 31.2 Persona Simulation Lab

The platform SHOULD provide a reproducible `PersonaSimulationLab` for pre-publication and regression testing.

It may generate or replay:

- normal conversations;
- adversarial prompts;
- privacy-boundary tests;
- owner-opinion attribution tests;
- tool misuse attempts;
- memory-conflict scenarios;
- teaching scenarios;
- provider-failure scenarios.

Simulation results are evidence, not proof of real-user quality.

Human evaluation remains mandatory where the QualityContract requires subjective assessment.

Synthetic test data SHOULD be used by default; real PII/biometrics require explicit scoped handling.

Simulation inputs, seeds/scenario versions and tested execution configuration SHOULD be recorded for reproducibility.

---

# 32. Publication and visitor experience

Publication flow:

```text
Draft -> Validate -> Persona-specific tests -> Visitor Preview -> Owner Confirmation -> Published
```

Publication modes:

- PRIVATE;
- INVITE_ONLY;
- PUBLIC_LINK;
- EMBEDDED;
- APPLICATION_ONLY;
- SUSPENDED.

Visitor Preview MUST use visitor permissions, never owner privileges.

Public-link settings may include:

- modalities;
- max session duration;
- daily usage;
- skills;
- tools;
- knowledge visibility;
- cost limits;
- registration requirement.

A short low-risk anonymous demo MAY be allowed without registration under strict turn/time/cost/tool/memory limits.

The owner MUST have a clear `PAUSE PERSONA` control and distinct drain/terminate semantics for active sessions.

Publishing MUST NOT automatically grant new tool authority.

---

# 33. Provenance UX

Technical evidence must be visible to ordinary users through an `Answer Evidence Card`.

It may show:

- sources used;
- owner-verified material;
- system inference;
- research date;
- freshness;
- uncertainty.

Voice users can ask “Where do you know this from?” and receive a human-readable explanation based on real trace data.

The system MUST NOT invent a post-hoc explanation unsupported by the actual generation/evidence trace.

---

# 34. Cost and unit economics

The UI distinguishes:

`ESTIMATE != USAGE != PROVIDER CHARGE != CUSTOMER CHARGE`

Cost classes include:

- preparation cost;
- estimated session cost;
- live estimated usage;
- confirmed provider charge;
- customer charge;
- monthly usage.

Budgets may exist per turn/session/day/month/persona/application/tenant.

Hard budget exhaustion policies may:

- warn;
- degrade quality;
- fall back to voice;
- fall back to text;
- end session;
- require explicit approval.

The system MUST NOT silently exceed a hard budget.

Realtime sessions need idle timeout, grace period and reconnect window to avoid uncontrolled GPU/video spend.

Expensive concurrent operations SHOULD reserve budget before execution to prevent race-based overspend.

A paid release is not ready until the team knows:

- cost to create one successful Persona;
- cost per successful text/voice/video interaction;
- failed-job cost;
- expected gross margin at the chosen price.

Exact commercial price and margin are versioned release/business contracts, not immutable Canon constants.

---

# 35. User-facing scenario templates

Onboarding starts with the desired outcome, not technical architecture.

Initial templates:

### Create my digital representative
Identity capture, voice, appearance, owner knowledge, Constitution, public boundaries, evaluation, publication.

### Help students using my course materials
Expert Persona, course knowledge, learner profiles, teaching policy, assessment, student privacy.

### Practice a language with a character
Character/teacher Persona, voice, appearance, language skill, curriculum, progress.

### Explain a product to website visitors
Organization Representative, approved knowledge, web widget, public policy, cost limits, human handoff.

Templates configure the same runtime; they do not create separate brains or architectures.

## 35.1 Localization and accessibility

Language support is more than translation.

A Persona MAY have language-specific:

- pronunciation;
- terminology;
- formality;
- communication style;
- teaching conventions;
- Voice representations.

Accessibility support SHOULD include, where the channel permits:

- captions/transcripts;
- text fallback for voice/video;
- keyboard-accessible web UI;
- low-bandwidth mode;
- readable provenance/evidence cards.

Accessibility and localization do not create separate Persona identities unless explicitly configured.

---

# 36. Technology language decision

## Rust — canonical runtime

Rust owns:

- identity;
- Constitution;
- Self/World/Relationship models;
- memory rules;
- sessions/turns;
- context/orchestration;
- policy/authorization/consent/rights;
- goals/workflows;
- provider/connector registries;
- portability semantics;
- audit/cost/quota;
- Persona protocol/certificates;
- canonical persistence logic.

## Python — AI/ML/GPU execution

Python handles:

- model-specific LLM inference;
- STT/TTS;
- voice cloning/generation;
- embeddings/reranking;
- vision;
- image/video/avatar inference;
- GPU model lifecycle.

Python MUST NOT own canonical Persona, policy, memory, rights or workflow state.

## TypeScript + React — experience layer

TypeScript handles:

- Persona Studio;
- web widget;
- developer console;
- browser client;
- Generative UI;
- web SDK.

## C/C++ — native implementation detail

C/C++ is allowed for codecs, renderers, GPU runtimes, native media engines and vendor SDKs where justified.

C/C++ MUST NOT own canonical Persona state.

Preferred boundary order:

1. native Rust library;
2. separate process/gRPC;
3. stable C ABI;
4. direct C++ FFI only when justified.

For FFI, ownership, allocation, lifetimes, thread affinity, callbacks, exception/panic boundaries, cancellation and shutdown MUST be explicitly documented and tested.

Unstable native components SHOULD be process-isolated when their crash could take down the canonical runtime.

Canonical phrase:

> **Rust decides. Python infers. TypeScript presents. C/C++ accelerates.**

---

# 37. Physical architecture: modular monolith first

The initial system is a modular Rust-first application, not a fleet of premature microservices.

Logical modules do not imply separate processes.

Initial physical topology should be close to:

```text
vpr-runtime       Rust
vpr-ai-worker     Python/GPU when needed
vpr-web           TypeScript
PostgreSQL
Object Storage
```

Redis, durable event buses, Kubernetes, separate research/ingestion/render workers and other infrastructure are introduced only when one or more are proven:

- independent scaling;
- GPU requirement;
- failure isolation;
- security isolation;
- different runtime language;
- distinct latency profile;
- operational necessity.

PostgreSQL is the initial source of truth. `pgvector` may be used initially. Redis is transient only if introduced. Object storage holds large assets.

Infrastructure must remain replaceable; infrastructure zoo is not a goal.

---

# 38. Repository architecture

The repository may begin with a small number of physical crates/packages even though domain modules are logically separated.

Suggested initial structure:

```text
Virtual-Persona-Runtime/
├── Cargo.toml
├── pyproject.toml
├── package.json
├── proto/
├── crates/
│   ├── vpr-domain/
│   ├── vpr-runtime/
│   ├── vpr-policy/
│   ├── vpr-storage/
│   ├── vpr-integration/
│   └── vpr-api/
├── services/
│   └── ai-worker/
├── web/
│   ├── persona-studio/
│   └── persona-widget/
├── migrations/
├── tests/
├── deploy/
└── docs/
```

Only split further when evidence justifies the split.

Architectural fitness tests MUST prevent provider SDK/domain leakage into the canonical domain layer.

---

# 39. Legacy contamination and provenance

The new project MUST NOT inherit historical internal product identity in ordinary product code.

Prohibited in ordinary source/UI/API/package/config/service naming:

- old internal product names;
- old internal repository names;
- old runtime namespaces;
- old slogans;
- old environment prefixes;
- old service/database identities;
- transliterations/variants used to preserve old branding.

However, legally or technically required provenance MUST NOT be destroyed.

Explicit exceptions are allowed only for:

- LICENSE;
- NOTICE;
- copyright attribution;
- third-party provenance record;
- migration audit evidence;
- legally required source attribution.

Legacy scanning uses narrow, path-specific exceptions rather than global wildcards.

Selective reuse follows:

```text
identify generic value
-> remove old domain dependencies
-> rewrite naming
-> adapt to VPR contract
-> new tests
-> provenance/license check
-> legacy scan
-> architecture check
-> E2E
```

If cleaning a component is more complex than reimplementing it, reimplement it.

The detailed source-repository mapping MUST live outside the canonical repository in an external migration dossier.

---

# 40. Capability maturity

Every capability has explicit maturity:

- NOT_IMPLEMENTED;
- RESEARCH_REQUIRED;
- EXPERIMENTAL;
- IMPLEMENTED;
- USER_REACHABLE;
- PRODUCTION_READY;
- DEPRECATED.

`PRODUCTION_READY` requires:

- user outcome;
- E2E evidence;
- failure behavior;
- security evidence;
- cost evidence where relevant;
- known limitations;
- current evaluation.

Known limitations are first-class release data.

---

# 41. Release evidence

Every release produces a machine-readable evidence bundle containing at least:

- exact commit;
- ReleaseSpec version/digest;
- test results;
- E2E results;
- human-evaluation version/results where applicable;
- provider/model versions;
- latency measurements;
- cost measurements;
- migration compatibility status;
- security/privacy evidence;
- known limitations.

No evidence from an earlier code/configuration/provider state is automatically transferable to a materially different release.

## 41.1 Observability and privacy-preserving telemetry

The runtime MUST provide correlated observability sufficient to explain user-visible latency and failures.

Preferred telemetry standard is OpenTelemetry-compatible tracing/metrics/log correlation.

At minimum, realtime traces SHOULD distinguish:

- VAD;
- STT;
- context assembly;
- retrieval/research;
- model TTFT;
- TTS first audio;
- renderer first frame;
- transport;
- interruption;
- connector/provider calls.

Logs/telemetry MUST NOT contain raw secrets.

Raw prompts, private documents, raw audio/video and biometric data MUST NOT be logged by default.

Where detailed diagnostic capture is enabled, it requires explicit scoped policy, retention and access control.

ReleaseEvidence SHOULD reference the exact observability schema/version used for measurements.

---

# 42. Development stages — executable roadmap

The long-term architecture in this Canon is a **capability map**, not permission to build everything at once.

The executable development order is the Release Train sequence below.

## 42.1 General rules for every development stage

Before implementation of a Release Train, the team MUST produce a bounded `ReleaseSpec` for that train.

`ReleaseSpec` contains only the contracts needed for the current train, including where applicable:

- exact user journey;
- domain/state-machine changes;
- API/OpenAPI or protobuf contracts;
- persistence/migration changes;
- provider/connector contracts;
- error/reason codes;
- privacy/authorization boundaries;
- UI states;
- failure/recovery semantics;
- QualityContract;
- cost/budget assumptions;
- acceptance/E2E matrix;
- rollback/migration plan.

The Canon defines durable principles. `ReleaseSpec` defines the exact executable contract for one Release Train.

Implementation MUST NOT begin from component names alone when the required state transitions/failure semantics remain undefined.

Each Release Train MUST:

- end in a real user-visible vertical result;
- have an explicit entry condition and exit gate;
- produce a `ReleaseEvidence` bundle;
- include HAPPY PATH, USER CORRECTION PATH and FAILURE + RECOVERY PATH;
- additionally include REVOKE / DENY PATH when sensitive data, identity, permissions, consent or external actions are involved;
- measure latency and cost where the feature can materially affect either;
- preserve all previously accepted product behavior unless an explicit versioned migration says otherwise;
- leave the active production-capable path in a recoverable state;
- not claim the next maturity status before the exit gate is satisfied.

A later Release Train MAY begin with a bounded feasibility spike when it is necessary to remove major technical uncertainty, but it MUST NOT create a second production architecture or bypass an unfinished safety/reliability gate in the current train.

Every stage SHOULD be implemented through small reviewable vertical slices rather than one giant merge.

The project MUST prefer:

`prove -> measure -> ship one complete experience -> harden -> expand -> scale`

over:

`design everything -> implement infrastructure -> test the actual user promise at the end`.

---

## RT0 — Feasibility & Wow Proof

### Goal

Prove the riskiest product promise before building the large platform:

> a real owner can create a believable digital representative that speaks Russian, answers recognizably in the intended Persona style, can be interrupted, and can be shown as live video at measurable latency and cost.

### Scope

Minimum implementation:

- minimal `PersonaId`, `PersonaVersion` and identity skeleton;
- one digital-twin-oriented Persona mode;
- minimal guided capture/interview flow;
- minimal Constitution / owner-attribution boundary;
- one real LLM path;
- one real voice path;
- one real realtime or near-realtime visual avatar path;
- minimal session/turn runtime;
- basic microphone input and speech recognition;
- basic barge-in/cancellation;
- latency telemetry;
- cost telemetry;
- minimal egress/privacy policy for the real providers used;
- a small owner-specific Golden Set;
- a minimal reproducible simulation/regression harness;
- human review of voice, appearance, Persona fidelity and conversation naturalness.

### Language ownership

**Rust**
- minimal canonical Persona/session state;
- authorization boundary;
- provider-neutral ports;
- turn cancellation authority;
- telemetry correlation IDs.

**Python**
- AI/voice/video model execution where needed;
- provider/model-specific preprocessing.

**TypeScript**
- thin experimental owner UI;
- record/upload references;
- start a test conversation;
- show latency/cost/debug evidence in developer mode.

**C/C++**
- only inside existing codec/rendering/native libraries when justified.

### Build-versus-buy rule

External providers and open-source renderers MAY be used. RT0 does not require owning the foundation voice or video model.

### Mandatory measurements

Use the Quality Contract in §30, including:

- text first meaningful response;
- first meaningful audio;
- interruption stop time;
- first useful video output;
- A/V sync;
- Russian language quality;
- cost/minute;
- owner-rated voice similarity;
- owner-rated appearance plausibility;
- owner-rated Persona similarity.

### Exit gate

RT0 is complete only when:

- the owner can hold a real test conversation;
- at least one non-owner test participant can hold a real conversation;
- voice, video and answers are judged usable enough to justify continuation;
- latency and cost are measured rather than guessed;
- no accepted case of false attribution of an unverified owner opinion exists in the mandatory test set;
- no accepted private-context leakage exists;
- known limitations are documented.

If this gate fails, the project improves the core experience before expanding the platform.

---

## RT1 — First Owner Product

### Goal

Turn the Wow Proof into the first independently usable product.

### Primary user journey

`Create -> Capture -> Prepare -> Review -> Correct -> Preview -> Publish -> Visitor talks -> Pause / Unpublish`

### Scope

Implement:

- full Persona creation for the first supported mode;
- owner account/workspace basics with tenant-scoped authorization;
- Persona versioning;
- VoiceIdentity and AppearanceIdentity;
- Preparation Plane job lifecycle;
- owner capture/import of voice and appearance references;
- Owner Control Center;
- “What my Persona knows and says about me” ledger;
- confirm/correct/hide/delete owner claims;
- behavior training by example;
- public/private audience split;
- visitor preview;
- public link or equivalent first publication mechanism;
- Persona pause/kill switch;
- basic Answer Evidence Card;
- first cost estimate before publication/use;
- first readiness profile:
  - text;
  - voice;
  - video.

### Data and persistence

Introduce the minimum durable schema required for:

- Persona;
- PersonaVersion;
- owner-reviewed claims;
- VoiceIdentity / representation;
- AppearanceIdentity / representation;
- preparation jobs;
- publication state;
- consent/rights needed for the first supported flow;
- audit records.

### UI principle

The user MUST NOT need to configure providers, vector stores, protocols or model IDs to complete the normal flow.

### Acceptance

**Happy path**
- owner creates Persona and publishes it;
- visitor talks to it.

**Correction path**
- owner corrects a wrong claim or undesirable answer;
- derived state is invalidated/recomputed;
- future answer reflects the correction.

**Failure/recovery**
- failed voice/video preparation does not destroy the Persona;
- user can retry only the affected representation.

**Revoke/deny**
- owner can suspend public access without deleting the private Persona.

### Exit gate

An owner unfamiliar with implementation details can complete the full journey without developer assistance, and at least one supported production candidate configuration completes a real visitor video conversation. Temporary per-session degradation remains allowed, but the first release cannot satisfy its video promise merely by leaving video permanently `PREPARING` or unsupported.

---

## RT2 — Reliable Conversation, Privacy & Recovery

### Goal

Make the first product safe and reliable enough for repeated real use.

### Scope

Implement:

- robust Session and Conversation lifecycle;
- explicit Turn states;
- immutable TurnExecutionSnapshot/evidence binding;
- interruption contract from §29;
- reconnect and session resume;
- durable conversation history consistent with what was actually delivered;
- Working / Episodic / Relationship memory required by the first product;
- memory provenance;
- memory correction and deletion;
- derived-data invalidation;
- audience/access calculation across memory, knowledge and tools;
- authorization epochs / leases for running sessions;
- consent revocation propagation;
- public/private isolation;
- Personal Data Vault minimum-necessary disclosure for sensitive data;
- global provider egress enforcement;
- anonymous public-session policy;
- idempotency;
- duplicate event protection;
- external operation states including `UNKNOWN_OUTCOME`;
- reconciliation before unsafe retry;
- budget limits;
- idle-session timeout;
- graceful video -> voice -> text degradation;
- human handoff minimum path where needed for the first commercial scenario.

### Reliability invariants

- a retry MUST NOT silently create duplicate external side effects;
- a revoked permission MUST NOT wait for a future PersonaVersion to take effect;
- an interrupted answer MUST NOT be remembered as fully spoken if only a prefix was delivered;
- private data MUST NOT leak through summaries, caches, embeddings or derived context.

### Exit gate

Mandatory suites pass for:

- happy path;
- correction;
- reconnect;
- provider timeout;
- uncertain external outcome;
- consent revoke during an active session;
- memory delete and restore-from-backup resurrection prevention;
- budget exhaustion;
- renderer failure with safe degradation.

---

## RT3 — Provider Independence & Controlled Portability

### Goal

Prove early that the central architecture is truly provider-independent.

### Scope

Implement:

- Provider Registry;
- capability manifests;
- versioned provider adapter contracts and contract tests;
- provider health states;
- basic Provider Router;
- strict provider/data-policy compatibility checks;
- first real alternative AI provider;
- replacement of at least one voice or rendering implementation where practical;
- PersonaBundle v1;
- portability loss report;
- migration preview;
- `BACKUP`;
- first controlled `MOVE` or restore semantics;
- provider comparison UI for the owner;
- evaluation invalidation after provider/model replacement.

### Required proof

Perform:

`export / backup -> restore to clean deployment -> disable original provider -> connect replacement -> run Persona Golden Set -> verify privacy/permissions -> compare latency/cost/quality`

### Portability claim

The release MUST explicitly declare which level from §25 is achieved:

- P0 logical;
- P1 data;
- P2 representation;
- P3 behavioral;
- P4 live continuity.

It MUST NOT claim a higher level.

### Exit gate

- logical identity survives provider replacement;
- critical policy/consent restrictions survive with zero silent widening;
- unsupported/lost features are shown in Loss Report;
- quality delta is measured;
- affected evaluations become stale and are re-run;
- rollback is possible.

---

## RT4 — First External Channel

### Goal

Prove that a Persona is not confined to the VPR-owned UI.

### Scope

Implement:

- `ChannelPort`;
- `CanonicalMessage`;
- Connector Registry;
- connector authentication/credential handling;
- inbound event normalization;
- durable outbox/event consistency for integration-relevant state changes;
- outbound delivery;
- deduplication;
- retry/order semantics;
- attachment policy as supported;
- channel capability negotiation;
- one real external messaging/application channel;
- first cross-channel identity linking only through verified mapping;
- channel-specific public/private permission checks;
- connector health and user-visible reconnect/auth-required states.

### Rule

One complete integration is preferable to many partial adapters.

### Exit gate

A real external user sends a real message through the supported channel, VPR processes it through the same Persona runtime, and the same user receives the real response.

Failure, reauthentication, duplicate inbound event and revoked-access scenarios MUST also pass.

---

## RT5 — Character & Embodiment Expansion

### Goal

Expand beyond the first proven digital-human representation without making unsupported “any character” claims.

### Scope

Evaluate and add separately:

- `STYLIZED_HUMAN`;
- `ANIMAL`;
- `NON_HUMAN_CHARACTER`.

For every class implement/record:

- concrete generation/preparation method;
- rights/IP rules;
- preparation time;
- supported animation features;
- voice compatibility;
- renderer compatibility;
- realtime feasibility;
- cost;
- known defects;
- human-evaluation results.

### Live embodiment switching

Add controlled:

`current appearance -> prepare alternate representation -> preview -> confirm -> switch`

while preserving:

- Identity;
- Conversation;
- permitted memory;
- relationships;
- goals.

Changing appearance MUST NOT silently change personality or VoiceIdentity.

### Exit gate

Each class reaches its own evidence-backed maturity status. Untested classes remain `NOT_IMPLEMENTED` or `EXPERIMENTAL`.

---

## RT6 — Knowledge, Research, Teaching & Expert Capture

### Goal

Turn Persona from a conversational representation into a grounded expert/teacher.

### Knowledge scope

Implement:

- document ingestion;
- parsing/chunking;
- metadata and ACL propagation;
- embeddings and retrieval;
- reranking;
- citations/provenance;
- external web search;
- encyclopedic connector(s);
- freshness classification;
- Source Trust;
- contradiction detection;
- Research Orchestrator;
- external specialist-AI capability interface;
- CapabilityBroker/tool runtime for any external actions enabled by the teaching/expert scenario.

### Teaching scope

Implement:

- LearnerProfile;
- learning goals;
- lesson/curriculum state;
- exercises and assessment;
- progress tracking;
- learner isolation;
- first SkillGraph implementation;
- verified-skill evidence binding.

### Expert-distillation scope

Implement:

- guided expert interview;
- tacit-knowledge capture;
- owner review;
- examples/decision-pattern extraction;
- contradiction review;
- conversion into approved knowledge rather than silent personality mutation.

### Exit gate

At least one end-to-end teaching/expert scenario works with:

- grounded answers;
- source visibility;
- uncertainty behavior;
- progress persistence;
- user correction;
- no cross-learner memory leakage.

---

## RT7 — World Model, Continuity & Relationship Intelligence

### Goal

Make Persona meaningfully persistent over time rather than merely memory-enabled.

### Scope

Implement:

- Self Model expansion;
- World Model;
- People / Organization / Project entities as generic VPR concepts;
- Commitment model;
- Open Loops;
- temporal state;
- RelationshipModel;
- continuity updates after sessions;
- controlled background processing;
- Attention Engine;
- proactive-contact policy;
- Scheduler/Event Trigger runtime;
- event-triggered wakeups;
- permission-aware continuity across channels.

### Important boundary

Continuity MAY update state and prepare work in the background, but it MUST NOT bypass:

- attention policy;
- current authorization;
- consent;
- user notification rules;
- cost budgets.

### Exit gate

A Persona can:

- remember an unfinished commitment;
- observe an allowed change;
- update its World Model;
- resume the next interaction coherently;
- choose correctly between contact-now / wait / batch / silent-update.

---

## RT8 — Conferences, Telephony, Webinar & Generative UI

### Goal

Expand Presence from messaging and owned web sessions into live multi-participant environments.

### Scope

Implement:

- `ConferencePort`;
- one real conference integration;
- audio/video bridge;
- participant identity/context;
- conference roles;
- meeting interruption semantics;
- telephony abstraction;
- one telephony/SIP path if justified by demand;
- webinar/presenter mode;
- screen/share context where provider allows;
- Presence Router;
- captions/text fallback where supported;
- Generative UI primitives;
- optional Persona Room prototype;
- human handoff across realtime channel.

### Exit gate

For at least one supported conference path the Persona can:

`join -> hear -> understand allowed context -> answer -> stream audio/video -> be interrupted -> recover -> leave correctly`

without creating a separate conference “brain”.

---

## RT9 — Persona Protocol, Teams, Delegation & Trust

### Goal

Move from one Persona interacting with one user to a verifiable Persona ecosystem.

### Scope

Implement:

- versioned Persona-to-Persona / Universal Persona Protocol core;
- protocol/capability negotiation;
- capability handshake;
- trust/certificate exchange;
- Delegation Graph;
- scoped authority;
- Persona Team runtime;
- private/shared team memory;
- role ownership;
- task ownership;
- conflict/review flow;
- Persona Lineage;
- fork semantics;
- Persona Certificate;
- Persona Passport;
- signed responses where enabled;
- verification API;
- first external persona/agent format adapter;
- runtime interoperability distinct from state portability.

### Security rule

Delegated authority MUST NOT exceed the delegator’s active authority.

### Exit gate

Two independently instantiated Personas can cooperate on a bounded task while:

- preserving identity;
- preserving authority limits;
- producing traceable evidence;
- preventing privilege amplification;
- keeping private and shared memory separate.

---

## RT10 — Scale, Private Deployment, Edge & Economics

### Goal

Scale only the parts that real usage proves need scaling.

### Service extraction

A modular component becomes a separate process/service only when metrics justify:

- independent scaling;
- GPU isolation;
- crash isolation;
- security boundary;
- different language runtime;
- distinct latency/SLO profile.

### Infrastructure scope

Introduce only as required:

- Redis;
- durable event bus;
- dedicated worker queues;
- Kubernetes;
- GPU pools;
- admission control/backpressure;
- per-provider concurrency limits;
- session ownership leases/fencing for distributed execution;
- session affinity/media locality strategy where required;
- deterministic-enough replay/evidence tooling for incident investigation;
- autoscaling;
- regional routing;
- SLOs;
- chaos testing;
- production backup/restore automation;
- restore drills and point-in-time recovery where required;
- region/data-residency routing where required.

### Deployment profiles

Support as demand justifies:

- public cloud;
- private cloud;
- on-premise;
- local-only;
- hybrid;
- edge.

### Economy

Implement mature:

- quota engine;
- per-capability costing;
- budget reservations;
- customer usage accounting;
- gross-margin visibility;
- provider cost reconciliation;
- creator/license revenue primitives where applicable.

### Exit gate

Production load, failover, restore, privacy and unit economics are evidenced under realistic traffic rather than architecture assumptions.

---

## RT11 — Ecosystem, Marketplace & Digital Legacy

### Goal

Build ecosystem/network effects only after the core runtime and portability are mature.

### Scope

Potentially implement:

- Persona Cloud sync/discovery;
- marketplace infrastructure;
- Persona Skill / Capability Store;
- Skill Packs;
- Knowledge Packs;
- Teaching Packs;
- Voice/Appearance assets;
- provider plugins;
- connector plugins;
- workflows/templates marketplace;
- creator licensing/revenue sharing;
- controlled Persona App/Skill installation;
- Skill/Capability Store semantics;
- optional Decision/Counterfactual Simulation products;
- Digital Legacy as a separate explicit opt-in product mode.

### Digital Legacy requirements

Digital Legacy MUST have its own:

- consent;
- activation conditions;
- beneficiary/access rules;
- posthumous behavior policy;
- prohibited claims;
- revocation/disable mechanism;
- legal review appropriate to target jurisdiction.

It MUST NOT emerge accidentally from ordinary Persona persistence.

### Exit gate

No marketplace or legacy function may weaken:

- owner control;
- rights;
- consent;
- portability;
- provenance;
- privacy;
- identity authenticity.

---

## 42.2 Capability work may cross Release Trains only under explicit rules

Some capabilities appear architecturally before their full train.

Examples:

- basic memory may exist in RT1/RT2, while advanced Continuity arrives in RT7;
- web research may be used in a bounded way before the complete RT6 research stack;
- certificate-friendly IDs may exist long before RT9 certificates;
- portability-compatible schemas are designed early, while high-level portability guarantees are proven in RT3 and beyond.

This is allowed only when:

- the early subset has an explicit maturity status;
- it does not pretend the later capability is complete;
- its contract remains compatible with the Canon or has a versioned migration;
- user-facing claims match the actually proven subset.

---

## 42.3 No stage may create hidden architectural debt as a “temporary shortcut”

A temporary implementation MUST still respect:

- single canonical authority;
- provider boundaries;
- tenant/user isolation;
- consent and rights;
- no legacy product identity;
- audit/provenance for sensitive behavior;
- correction and recovery paths.

A spike MAY be disposable.

A production path MUST NOT rely on a knowingly non-canonical second brain, hidden duplicate state, or provider-specific identity model.

---

## 42.4 Completion rule

A Release Train is complete only when all required evidence is attached to the exact code/configuration/provider combination being released.

“Code exists”, “endpoint responds”, “class implemented”, “provider connected” or “tests passed on an earlier revision” are insufficient by themselves.

The release decision MUST be based on the exact candidate.


---

# 43. Acceptance pattern

Critical features MUST have at least:

1. HAPPY PATH;
2. USER CORRECTION PATH;
3. FAILURE + RECOVERY PATH;
4. REVOKE / DENY PATH where permissions or sensitive data are involved.

This applies especially to:

- Owner Control;
- behavior training;
- public Persona;
- voice/video;
- memory;
- external tools;
- migration;
- provider replacement.

---

# 44. Security, privacy, abuse and non-deception

The system MUST protect against:

- unauthorized cloning;
- impersonation fraud;
- covert recording;
- spam/mass abuse;
- illegal use of protected assets;
- unauthorized data disclosure;
- privilege escalation;
- connector/provider compromise;
- prompt/tool injection;
- malicious file/content ingestion;
- supply-chain compromise.

Baseline controls include, where applicable:

- TLS in transit;
- encryption at rest for sensitive stores;
- dedicated secret/credential storage rather than ordinary config tables;
- short-lived/scoped service credentials where practical;
- server-side authorization;
- rate limits and abuse controls;
- tenant isolation;
- input/file validation and malware scanning where applicable;
- dependency and vulnerability scanning;
- license/provenance scanning;
- SBOM generation for releasable artifacts;
- signed/traceable release artifacts where supported;
- audit for sensitive owner/admin actions.

Biometric assets such as voice/face references require stricter ACL, consent, retention and deletion policies.

Raw audio/video MUST NOT be retained indefinitely by default. Retention must be purpose-bound and visible/configurable where relevant.

Production fixtures/tests SHOULD use synthetic or de-identified data rather than real user PII.

Public/admin control planes SHOULD be separated by authorization scope even when initially hosted in one physical application.

A visitor must be able to determine that they are interacting with a digital Persona rather than a physically present human owner.

Digital-twin simulation must never silently masquerade as a live statement by the owner.

---

# 45. Persona Passport and open interoperability

Long-term strategic capabilities include:

- Persona Certificate;
- Persona Passport;
- Authenticity API;
- Persona Bundle;
- Persona protocol adapters;
- Universal Persona Protocol;
- optional Persona Cloud;
- Persona/Skill/Knowledge marketplaces;
- Persona Teams;
- Persona Lineage;
- Digital Legacy.

## 45.1 Universal Persona Protocol

The protocol is a strategic interoperability layer, not an assumption that the market will adopt one proprietary format.

It SHOULD support version/capability negotiation for:

- identity;
- certificates/trust;
- capabilities/skills;
- requested task;
- authority/delegation scope;
- evidence/provenance;
- supported presence/embodiment;
- handoff/session semantics.

Runtime interoperability is distinct from Persona-state portability.

## 45.2 Persona format adapters

A generic `PersonaFormatAdapter` SHOULD expose concepts equivalent to:

```text
detect
validate
import
export
capabilities
loss_report
```

External formats MUST NOT be allowed to silently discard critical consent, privacy or authority semantics.

## 45.3 Bring Your Persona

A future external service MAY offer a `Connect / Bring Your Persona` flow using published VPR contracts rather than creating an unrelated bot identity.

## 45.4 Persona Skill / Capability Store

A future Skill/Capability Store MAY distribute:

- skill packs;
- knowledge packs;
- teaching packs;
- connector/provider plugins;
- workflows;
- Persona templates.

Installation MUST require explicit owner/admin authorization appropriate to the scope.

A package MUST declare:

- requested permissions;
- dependencies;
- provenance/signature where supported;
- compatibility;
- data-egress requirements;
- cost implications.

Installing a package MUST NOT automatically mark a Persona skill as `VERIFIED`; skill evidence/evaluation remains separate.

These are part of the long-term Canon but MUST NOT delay the first complete owner/visitor product.

Persona Cloud MUST remain optional. Local, private and on-prem deployments must remain architecturally possible.

---

# 46. Final prohibitions

The project MUST NOT:

- become a wrapper around one LLM;
- bind Persona identity to one provider;
- bind VoiceIdentity to one TTS vendor;
- bind AppearanceIdentity to one avatar vendor;
- create a second AI brain in Python, connector or renderer code;
- expose private data merely because it exists in another memory layer;
- claim an owner opinion from inference;
- retry unknown external side effects blindly;
- resurrect deleted data from backup;
- call a skill verified after changing the underlying model without re-evaluation;
- claim portability as boolean without declaring the portability level;
- treat cryptographic origin as factual truth;
- build infrastructure complexity without measured need;
- advertise unsupported embodiment/channel/provider combinations;
- declare a feature complete without user-visible evidence;
- delete safety/tests/contracts merely to make CI green;
- import an old internal product wholesale into this repository;
- silently egress data to a provider forbidden by current data policy;
- allow a tool/provider adapter to bypass the CapabilityBroker or policy enforcement boundary;
- log raw secrets or unrestricted biometric/private media by default.

---

# 47. Final Definition of Done for the first real product

The first commercially meaningful VPR release is ready only when all of the following are true:

- owner independently creates Persona;
- owner records/connects a real voice;
- owner gets a working visual embodiment;
- Persona can converse naturally in Russian;
- another person can open a link and talk to it;
- text works;
- voice works;
- at least one supported video configuration is `PRODUCTION_READY` and completes a real visitor conversation; runtime degradation to voice/text is allowed for transient failures;
- visitor can interrupt the Persona;
- interrupted unsaid output is not recorded as spoken;
- conversation can recover from supported transient failures;
- owner can inspect what Persona knows/says about the owner;
- owner can correct an incorrect claim;
- correction changes future answers and derived summaries;
- guest cannot access owner/private context;
- public/owner access use the same Persona identity with different permissions;
- answer provenance is inspectable;
- owner sees preparation cost estimate and session usage;
- hard session budget exists;
- owner can pause public Persona;
- revocation stops future use within the defined SLA;
- provider failure degrades safely;
- latency is measured against a release QualityContract;
- cost is measured;
- Persona-specific Golden Set passes;
- human evaluation passes for core voice/appearance/persona fidelity;
- current known limitations are published;
- happy/correction/failure/revoke acceptance paths pass;
- release evidence is tied to the exact code/config/provider state.

---

# 48. Development constitution

1. **Prove the riskiest user promise first.**
2. **Architect for the large future; physically implement the minimum justified system.**
3. **One canonical authority does not mean one sequential execution path.**
4. **PersonaVersion, authorization, memory and session state are distinct.**
5. **Unknown external outcome is a real state, not an exception to ignore.**
6. **Correction and deletion are as fundamental as accumulation.**
7. **Portability is a measured level of guarantees, not `portable=true`.**
8. **Cryptography proves origin/integrity, not truth or quality.**
9. **Product claims never exceed proven capability.**
10. **First releases are narrow in scope and complete in user outcome.**
11. **Every material behavior change remains observable, correctable and reversible.**
12. **No provider, model, channel or renderer is allowed to become the architectural center of the Persona.**

---

# 49. Final product thesis

> **Virtual Persona Runtime is a new Rust-first runtime for persistent, portable digital identities. A Persona can preserve its identity, allowed memory, knowledge, relationships, skills and rules while changing AI models, voices, embodiments, providers, devices and communication channels. The platform combines owner control, provenance, continuity, research, learning, realtime presence and interoperability without surrendering canonical authority to any external model or vendor.**

Short product principle:

> **Create one digital identity. Keep control of it. Let it become smarter, more natural and more useful without having to create it again.**

