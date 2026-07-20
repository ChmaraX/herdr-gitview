# Task for reviewer

Perform the thermo-nuclear code quality review (per the loaded skill) on the entire herdr-gitview codebase at /Users/adamchmara/projects/herdr-gitview. This is a Rust ratatui TUI plugin for the herdr terminal workspace manager: two panes (file list + diff preview) communicating over a unix socket, with git operations, an nvim editor loop on the preview PTY, commit history browsing, review notes sent to AI agent panes, and popup dialogs.

Since this reviews the whole codebase (not a diff), audit every file under src/ (14 files, ~6k lines) plus tests/. Pay special attention to:
- src/list/mod.rs and src/list/app.rs (the run loop has grown many responsibilities: IPC, popups, editor probes, notes)
- src/preview/mod.rs and src/preview/app.rs (similar growth: diff worker, editor suspension, popups, notes store, selection)
- duplication between the two panes (input threads, IPC forwarders, popup answer polling, repo resolution)
- the ipc.rs message enums vs how handlers are scattered
- abstraction quality of render.rs / highlight.rs / git.rs
- file sizes approaching or exceeding 1k lines
- spaghetti-condition growth in the event loops (mode checks, busy checks, modal checks stacked in the key-handling branches)

Deliver: (1) a prioritized list of code-judo restructurings with concrete before/after shapes, (2) smaller local issues worth fixing, (3) anything that looks like a real bug. Do NOT modify any files — review only. Be harsh; measure twice.

---
**Output:**
Write your findings to exactly this path: /tmp/gitview-thermo-review.md
This path is authoritative for this run.
Ignore any other output filename or output path mentioned elsewhere, including output destinations in the base agent prompt, system prompt, or task instructions.

## Acceptance Contract
Acceptance level: attested
Completion is not accepted from prose alone. End with a structured acceptance report.

Criteria:
- criterion-1: Return concrete findings with file paths and severity when applicable

Required evidence: review-findings, residual-risks

Finish with a fenced JSON block tagged `acceptance-report` in this shape:
Use empty arrays when no items apply; array fields contain strings unless object entries are shown.
`criteriaSatisfied[].status` must be exactly one of: satisfied, not-satisfied, not-applicable.
`commandsRun[].result` must be exactly one of: passed, failed, not-run.
`manualNotes` and `notes` are optional strings; an empty string means no note and does not satisfy `manual-notes` evidence.
```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "specific proof"
    }
  ],
  "changedFiles": [
    "src/file.ts"
  ],
  "testsAddedOrUpdated": [
    "test/file.test.ts"
  ],
  "commandsRun": [
    {
      "command": "command",
      "result": "passed",
      "summary": "short result"
    }
  ],
  "validationOutput": [
    "validation output or concise summary"
  ],
  "residualRisks": [
    "none"
  ],
  "noStagedFiles": true,
  "diffSummary": "short description of the diff",
  "reviewFindings": [
    "blocker: file.ts:12 - issue found, or no blockers"
  ],
  "manualNotes": "anything else the parent should know"
}
```