# ADR 0017 — Theme activation uses sealed immutable generations

- **Status:** Accepted (2026-08-29)
- **Deciders:** helm maintainers, repo owner
- **Supersedes / Superseded by:** Supplements ADR 0005; #22 owns the physical
  publication transaction and #132 owns session/scope lifecycle. Supersedes
  ADR 0005's mixed-generation interruption allowance when activation
  generations are implemented.

## Context

ADR 0005/SPEC 0002 protect individual generated files, but deliberately do not
make a set of files a transaction. That is insufficient once target launch
configuration needs a stable identity: a mutable `helm/generated` directory
cannot prove which palette, target catalogue, or output bytes a process used.

## Decision

1. Helm creates its generated root with mode 0700. An apply first materializes a new **sealed generation** at
   `$XDG_CONFIG_HOME/helm/generated/generations/<generation-id>/`. Its manifest
   names every normalized output path and SHA-256 digest, and binds the
   canonical palette digest, exact catalogue/templates/rendering inputs, and
   launch-profile inputs. The manifest has a
   versioned, canonical encoding and lexicographically sorted normalized paths;
   it is itself covered by the seal. A sealed generation is never altered,
   including by repair or rollback.
2. `current` is a regular UTF-8 file containing exactly one validated generation
   id and newline; it is not a symlink. A launcher resolves it once while holding
   the shared generation lock, validates the manifest and digests, creates and
   fsyncs a
   lease, and launches using only paths below that selected generation. A process
   already launched remains pinned to that selection; a later apply changes only
   future launches.
3. All participants open the same descriptor-relative, no-follow `activation.lock`
   under Helm's generated root and use advisory shared/exclusive locks on that
   inode. Kernel release on process death is stale-writer recovery; no process
   steals a lock from a live owner. Writers take the exclusive lock. They render and fsync the complete staged
   tree, fsync its manifest, rename the staged directory to its final immutable
   name, fsync `generations/`, atomically replace `current` with a fsynced
   no-follow sibling, then fsync the parent. No reload/activation occurs until
   this record/tree pair is durable. Recovery discards unsealed staging and
   rejects a `current` record whose tree or manifest fails validation. It never
   promotes a partial/staging tree, and preserves an invalid sealed tree for
   explicit diagnosis rather than guessing which bytes are safe to delete.
4. A pointer switch does **not** reload, signal, or retheme an existing process.
   Reload is legal only through a future generation-aware owned-process protocol
   that proves the process selected that generation; foreign/direct launches are
   not generation-selected.
5. Rollback validates a retained sealed generation and performs only the pointer
   commit above. It does not rewrite old bytes or migrate an already-running
   process.
6. Every lease records generation id, PID, Linux process start-time, boot ID and
   owner identity. Before spawning, a launcher validates under the shared lock,
   fsyncs its lease, and only then executes.
   Reclamation under the exclusive lock removes a lease only when the boot ID,
   PID, or start-time no longer matches `/proc`; a live lease prevents deletion.
   GC retains `current` plus the two most recent unleased generations. On any
   inability to prove liveness, it retains rather than deletes. At retention
   exhaustion it refuses a new apply rather than reclaiming a protected tree.

## Consequences

- Launches are reproducible and an old generation cannot resolve later bytes.
- A crash cannot make `current` name an unsealed or mismatched tree; it can leave
  harmless staging or a valid unpointed orphan. A missing/corrupt pointer,
  missing tree, or digest mismatch fails closed; recovery never guesses newest.
- “Sealed” means Helm never mutates a published generation and consumers verify
  before use; it does not claim protection from another same-UID actor.
- The contract does not promise atomic live upgrade across clients, automatic
  session/scope cleanup, or a cross-file transaction outside the generation
  publication protocol. Those remain #22 and #132 work.

## Guard

- *Planned (#131):* hostile fixtures for mismatched manifests, crash points,
  concurrent writers, stale lease identity reuse, and old-generation launch.
- *Planned (#22):* end-to-end transaction and exactly-once reload fan-out.
- *Planned (#132):* session/scope lifecycle ownership and logout/restart rules.

## Needs a human

No further decision is required: the owner authorized the pin-on-launch policy
on 2026-08-29. Package source, session lifecycle and live-client upgrade remain
outside this ADR.
