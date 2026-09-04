# Relay

<!-- impeccable:product-schema 1 -->

## Platform

Native Linux desktop.

## Stack

Rust, GTK4, VTE, and GtkSourceView, explicitly confirmed by Antho. The Rust
engine, bus and CLI are retained from Relay-2. The native presentation is new.

## Users

Antho directs coding agents across local Git repositories, often running
builders and reviewers together.

## Product Purpose

Keep agent terminals, tasks, worktrees, messages and approvals in one desktop
workspace. Make it easy to see which session needs attention without losing work.

## Capabilities and Constraints

- Relay-2 is the visual and interaction reference.
- Performance and agent coordination are priorities: guardrails, mailbox,
  separate builder/reviewer authority and safe worktree ownership.
- The engine owns processes and persistent state. Closing a window ends its
  subscriptions, not its agents.
- V3's future roadmap is imported separately and is not a completion claim.
- No Tauri, webview, client-side polling or automatic destructive cleanup.

## Evidence on Hand

Pinned source revisions and imported documentation are listed in docs/SOURCES.md.
Measured performance and parity limits belong in docs/VERIFICATION.md.
