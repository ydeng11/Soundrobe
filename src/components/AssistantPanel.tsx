import React, { useState, useRef, useEffect, useCallback } from "react";
import type {
  AssistantEvent,
  AssistantActionBatch,
  TrackData,
  TrackUndoSnapshot,
  ExtraTagUndoSnapshot,
} from "../shared/desktop-api";
import { ScanProgressBar } from "./ScanProgressBar";

interface StatusDetail {
  icon: string;
  text: string;
}

interface AssistantApplyProgress {
  batchId: string;
  phase: "preflight" | "writing" | "verifying";
  current: number;
  total: number;
  message: string;
}

type AssistantStatus =
  | "sending"
  | "thinking"
  | "looking_up"
  | "applying_changes"
  | "ready_for_review"
  | "responded"
  | "completed"
  | "failed";

interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
  type?: "text" | "tool_running" | "tool_result" | "action_batch" | "error";
  batch?: AssistantActionBatch;
  /** Status indicator for assistant reply messages */
  status?: AssistantStatus;
  /** Accumulated backend trace entries (collapsible) */
  details?: StatusDetail[];
  /** Original user prompt stored on assistant reply for retry-from-failure */
  userMessage?: string;
  /** Native batch represented by this preview message. */
  actionBatchId?: string;
}

interface AssistantPanelProps {
  isOpen: boolean;
  onClose: () => void;
  onOpenSettings: () => void;
  /** Whether an LLM API key is configured (backend resolves it from ConfigState). */
  keyConfigured: boolean;
  model?: string;
  libraryPath: string | null;
  activeAlbumPath: string | null;
  selectedTrackPaths: string[];
  allTracks: TrackData[];
  allAlbums: Array<{ path: string; name: string; artistHint: string; albumHint: string; trackCount: number }>;
  autonomous: boolean;
  onRefreshRequest: () => void;
  onAssistantRunTask?: (
    task: "auto_tag" | "audit",
    trackPaths: string[],
  ) => Promise<void> | void;
  onAssistantApplyUndo?: (
    description: string,
    snapshots: TrackUndoSnapshot[] | ExtraTagUndoSnapshot[],
    kind: "tag-update" | "extra-tag-update",
  ) => void;
}

const SUGGESTED_PROMPTS = [
  "Summarize my library",
  "Find tracks missing genres",
  "Search MusicBrainz for this album",
  "Organize this folder by extension",
] as const;

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

function verificationFailureDetail(data: unknown): string | null {
  if (!data || typeof data !== "object" || !("verification" in data)) return null;
  const verification = data.verification;
  if (
    !verification ||
    typeof verification !== "object" ||
    !("failures" in verification) ||
    !Array.isArray(verification.failures)
  ) {
    return null;
  }
  const details = verification.failures
    .map((failure) => {
      if (!failure || typeof failure !== "object") return null;
      const path =
        "trackPath" in failure && typeof failure.trackPath === "string"
          ? failure.trackPath
          : null;
      const error =
        "error" in failure && typeof failure.error === "string"
          ? failure.error
          : "Verification failed";
      return path ? `${path}: ${error}` : error;
    })
    .filter((detail): detail is string => detail !== null);
  return details.length > 0 ? details.join("\n") : null;
}

export function AssistantPanel({
  isOpen,
  onClose,
  onOpenSettings,
  keyConfigured,
  model,
  libraryPath,
  activeAlbumPath,
  selectedTrackPaths,
  allTracks,
  allAlbums,
  autonomous,
  onRefreshRequest,
  onAssistantRunTask,
  onAssistantApplyUndo,
}: AssistantPanelProps) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [inputText, setInputText] = useState("");
  const [sending, setSending] = useState(false);
  const sendingRef = useRef(false);
  const [pendingBatches, setPendingBatches] = useState<AssistantActionBatch[]>([]);
  const [applying, setApplying] = useState(false);
  const [applyProgress, setApplyProgress] = useState<AssistantApplyProgress | null>(null);
  const applyingBatchIdRef = useRef<string | null>(null);
  const [clearing, setClearing] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [sessionNumber, setSessionNumber] = useState<string | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const [expandedMsgIndex, setExpandedMsgIndex] = useState<number | null>(null);
  /** Ref for the in-flight assistant message — avoids race between setMessages batching and backend events */
  const pendingMsgRef = useRef<ChatMessage | null>(null);

  const resizeInput = useCallback((element: HTMLTextAreaElement) => {
    element.style.height = "40px";
    element.style.height = `${Math.min(112, Math.max(40, element.scrollHeight))}px`;
  }, []);

  useEffect(() => {
    if (inputRef.current) resizeInput(inputRef.current);
  }, [inputText, resizeInput]);

  // Scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // On open: focus input, eagerly init runtime so the session number is
  // available immediately instead of waiting for the first message.
  useEffect(() => {
    if (!isOpen) return;
    console.log("[Assistant] Panel opened");
    inputRef.current?.focus();
    initRuntimeAndRefresh();
  }, [isOpen]);

  const initRuntimeAndRefresh = useCallback(async () => {
    try {
      await window.api.assistantInitRuntime();
      console.log("[Assistant] Runtime initialized");
    } catch (e) {
      console.log("[Assistant] Runtime init skipped:", e);
    }
    refreshSessionNumber();
  }, []);

  const refreshSessionNumber = useCallback(async () => {
    try {
      const s = await window.api.getCurrentSession();
      console.log("[Assistant] getCurrentSession:", s);
      if (s?.sessionNumber) setSessionNumber(s.sessionNumber);
    } catch (e) {
      console.log("[Assistant] getCurrentSession error:", e);
    }
  }, []);

  /**
   * Update the in-flight pending message ref synchronously (not subject to
   * React batching). The ref content is merged into `messages` on completion
   * and rendered via displayMessages.
   */
  const updatePendingMsg = useCallback(
    (updates: {
      status?: AssistantStatus;
      content?: string;
      detail?: StatusDetail;
      actionBatchId?: string;
    }) => {
      const m = pendingMsgRef.current;
      if (!m) return;
      if (updates.status) m.status = updates.status;
      if (updates.content !== undefined) m.content = updates.content;
      if (updates.actionBatchId) m.actionBatchId = updates.actionBatchId;
      if (updates.detail) {
        m.details = [...(m.details || []), updates.detail];
      }
      // Signal React to re-render (messages identity changes even though ref mutates)
      setMessages((prev) => [...prev]);
    },
    [],
  );

  const updateBatchMsg = useCallback(
    (actionBatchId: string, updates: { status: AssistantStatus; detail?: StatusDetail }) => {
      setMessages((prev) =>
        prev.map((message) =>
          message.actionBatchId === actionBatchId
            ? {
                ...message,
                status: updates.status,
                details: updates.detail
                  ? [...(message.details || []), updates.detail]
                  : message.details,
              }
            : message,
        ),
      );
    },
    [],
  );

  const loadPendingBatches = useCallback(async () => {
    try {
      const batches = await window.api.assistantGetBatches();
      setPendingBatches(batches);
    } catch {
      // Ignore
    }
  }, []);

  // Listen for assistant events
  useEffect(() => {
    if (!isOpen) return;
    const unsub = window.api.onAssistantEvent((event: AssistantEvent) => {
      switch (event.type) {
        case "tool_running":
          updatePendingMsg({ status: "thinking", detail: { icon: "•", text: event.message } });
          break;
        case "tool_result":
          updatePendingMsg({ status: "looking_up", detail: { icon: "↳", text: event.message } });
          break;
        case "action_batch_created":
          const actionBatchId =
            event.data &&
            typeof event.data === "object" &&
            "actionBatchId" in event.data &&
            typeof event.data.actionBatchId === "string"
              ? event.data.actionBatchId
              : undefined;
          updatePendingMsg({
            status: "ready_for_review",
            content: event.message,
            detail: { icon: "•", text: event.message },
            actionBatchId,
          });
          if (actionBatchId) {
            loadPendingBatches();
          }
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "action_batch_progress":
          if (
            event.data &&
            typeof event.data === "object" &&
            "batchId" in event.data &&
            typeof event.data.batchId === "string" &&
            event.data.batchId === applyingBatchIdRef.current &&
            "phase" in event.data &&
            (event.data.phase === "preflight" ||
              event.data.phase === "writing" ||
              event.data.phase === "verifying") &&
            "current" in event.data &&
            typeof event.data.current === "number" &&
            "total" in event.data &&
            typeof event.data.total === "number"
          ) {
            setApplyProgress({
              batchId: event.data.batchId,
              phase: event.data.phase,
              current: event.data.current,
              total: event.data.total,
              message: event.message,
            });
          }
          break;
        case "action_batch_applied":
          if (
            event.data &&
            typeof event.data === "object" &&
            "batchId" in event.data &&
            typeof event.data.batchId === "string"
          ) {
            const verificationRequired =
              "verificationRequired" in event.data &&
              event.data.verificationRequired === true;
            const verified =
              "verification" in event.data &&
              event.data.verification !== null &&
              typeof event.data.verification === "object" &&
              "status" in event.data.verification &&
              event.data.verification.status === "verified";
            const warnings =
              "verification" in event.data &&
              event.data.verification !== null &&
              typeof event.data.verification === "object" &&
              "warnings" in event.data.verification &&
              Array.isArray(event.data.verification.warnings)
                ? event.data.verification.warnings.filter(
                    (warning): warning is string => typeof warning === "string"
                  )
                : [];
            const informational =
              "verification" in event.data &&
              event.data.verification !== null &&
              typeof event.data.verification === "object" &&
              "informational" in event.data.verification &&
              Array.isArray(event.data.verification.informational)
                ? event.data.verification.informational.filter(
                    (message): message is string => typeof message === "string"
                  )
                : [];
            updateBatchMsg(event.data.batchId, {
              status: verificationRequired && !verified ? "failed" : "completed",
              detail:
                verificationRequired && !verified
                  ? { icon: "!", text: "Native readback verification was not confirmed." }
                  : warnings.length > 0
                    ? { icon: "!", text: warnings[0] }
                    : informational.length > 0
                      ? { icon: "✓", text: informational[0] }
                      : { icon: "✓", text: event.message },
            });
            if (event.data.batchId === applyingBatchIdRef.current) {
              applyingBatchIdRef.current = null;
              setApplyProgress(null);
            }
          }
          loadPendingBatches();
          setSending(false);
          onRefreshRequest();
          break;
        case "action_batch_rejected":
          if (
            event.data &&
            typeof event.data === "object" &&
            "batchId" in event.data &&
            event.data.batchId === applyingBatchIdRef.current
          ) {
            applyingBatchIdRef.current = null;
            setApplyProgress(null);
          }
          updatePendingMsg({ detail: { icon: "×", text: event.message } });
          loadPendingBatches();
          break;
        case "action_batch_failed":
          const failureDetail = verificationFailureDetail(event.data);
          if (
            event.data &&
            typeof event.data === "object" &&
            "batchId" in event.data &&
            typeof event.data.batchId === "string"
          ) {
            if (event.data.batchId === applyingBatchIdRef.current) {
              applyingBatchIdRef.current = null;
              setApplyProgress(null);
            }
            updateBatchMsg(event.data.batchId, {
              status: "failed",
              detail: {
                icon: "!",
                text: failureDetail
                  ? `${event.message}\n${failureDetail}`
                  : event.message,
              },
            });
          } else {
            updatePendingMsg({ status: "failed", detail: { icon: "!", text: event.message } });
          }
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "message":
          // A prose reply is an answer, not evidence that requested work ran.
          updatePendingMsg({ status: "responded", content: event.message });
          setSending(false);
          pendingMsgRef.current = null;
          refreshSessionNumber();
          break;
        case "error":
          updatePendingMsg({ status: "failed", content: event.message, detail: { icon: "!", text: event.message } });
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "completed":
          if (/\b(couldn'?t complete|maximum step limit|no action was performed|malformed tool call)\b/i.test(event.message || "")) {
            updatePendingMsg({ status: "failed", content: event.message || "Incomplete.", detail: { icon: "!", text: event.message || "Incomplete." } });
          } else {
            updatePendingMsg({ status: "completed", content: event.message || "Completed." });
          }
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "cancelled":
          updatePendingMsg({ status: "failed", detail: { icon: "■", text: event.message || "Cancelled" } });
          setSending(false);
          pendingMsgRef.current = null;
          break;
      }
    });
    return () => unsub();
  }, [
    isOpen,
    onRefreshRequest,
    refreshSessionNumber,
    updatePendingMsg,
    updateBatchMsg,
    loadPendingBatches,
  ]);

  useEffect(() => {
    if (!isOpen) {
      applyingBatchIdRef.current = null;
      setApplyProgress(null);
    }
    return () => {
      applyingBatchIdRef.current = null;
    };
  }, [isOpen]);

  // Fallback cancellation timer: the native tool loop owns a 600-second
  // deadline, so this renderer safety net must remain beyond it.
  useEffect(() => {
    sendingRef.current = sending;
    if (!sending) return;
    const timerId = setTimeout(async () => {
      if (sendingRef.current) {
        console.warn("[Assistant] Fallback cancellation timer fired (630 s)");
        try { await window.api.assistantCancel(); } catch { /* runtime may not exist */ }
      }
    }, 630_000);
    return () => clearTimeout(timerId);
  }, [sending]);

  // Load batches on mount
  useEffect(() => {
    if (isOpen) {
      loadPendingBatches();
    }
  }, [isOpen, loadPendingBatches]);

  /**
   * Start a fresh conversation: reset the renderer transcript and reset the
   * native session (a new current session in cache.db).
   * Shared by the /clear command and the header "New chat" button.
   */
  const handleNewConversation = useCallback(async () => {
    if (sending || applying || clearing) return false;
    setClearing(true);
    try {
      await window.api.assistantClear();
      setInputText("");
      setEditingIndex(null);
      setSending(false);
      pendingMsgRef.current = null;
      setMessages([
        {
          role: "system",
          content: "Session cleared. Start a new conversation.",
          type: "text",
        },
      ]);
      setSessionNumber(null);
      await refreshSessionNumber();
      return true;
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      setMessages((prev) => [
        ...prev,
        {
          role: "system",
          content: `Failed to start a new conversation: ${detail}`,
          type: "error",
        },
      ]);
      return false;
    } finally {
      setClearing(false);
    }
  }, [applying, clearing, refreshSessionNumber, sending]);

  const handleSend = async () => {
    const text = inputText.trim();
    if (!text || sending || clearing) return;

    if (text === "/clear") {
      console.log("[Assistant] /clear — resetting session");
      if (await handleNewConversation()) {
        console.log("[Assistant] Session reset complete");
      }
      return;
    }

    setInputText("");
    setEditingIndex(null);
    setSending(true);
    console.log(`[Assistant] Sending: "${text.slice(0, 60)}"`);
    // Set ref synchronously so the event handler can update it immediately
    pendingMsgRef.current = {
      role: "assistant",
      content: "",
      status: "sending",
      details: [],
      userMessage: text,
    };
    setMessages((prev) => [
      ...prev,
      { role: "user", content: text, type: "text" },
      pendingMsgRef.current!,
    ]);

    try {
      // The API key is resolved server-side from ConfigState — never send
      // the redacted/masked renderer copy to the backend.
      await window.api.assistantSend({
        message: text,
        apiKey: "",
        model,
        libraryPath,
        activeAlbumPath,
        selectedTrackPaths,
        tracks: allTracks,
        albums: allAlbums,
        autonomous,
      });
      console.log(`[Assistant] assistantSend resolved`);
      refreshSessionNumber();
    } catch (error) {
      updatePendingMsg({
        status: "failed",
        detail: {
          icon: "⚠️",
          text: `Failed to send: ${error instanceof Error ? error.message : String(error)}`,
        },
      });
      setSending(false);
      pendingMsgRef.current = null;
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // Enter during an input-method composition (e.g. confirming a candidate
    // word in a Chinese IME) must go to the input, not send the message.
    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      !e.nativeEvent.isComposing &&
      e.keyCode !== 229
    ) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleCancel = async () => {
    try {
      await window.api.assistantCancel();
      updatePendingMsg({
        status: "failed",
        detail: { icon: "■", text: "Cancelled by user" },
      });
      setSending(false);
      pendingMsgRef.current = null;
    } catch {
      // Ignore
    }
  };

  const handleApply = async (batchId: string) => {
    applyingBatchIdRef.current = batchId;
    setApplyProgress(null);
    setApplying(true);
    try {
      const result = await window.api.assistantApplyActions(batchId);
      const detail =
        !result.success && Array.isArray(result.results)
          ? result.results
              .map(
                (r: { trackPath?: string; error?: string }) =>
                  `  • ${r.trackPath ?? "?"}: ${r.error ?? "unknown"}`,
              )
              .join("\n")
          : "";
      if (result.success) {
        // Push undo snapshots if available
        if (result.undoSnapshots && result.undoSnapshots.length > 0) {
          onAssistantApplyUndo?.("Assistant tag edit", result.undoSnapshots, "tag-update");
        }
        if (result.extraUndoSnapshots && result.extraUndoSnapshots.length > 0) {
          onAssistantApplyUndo?.("Assistant extra tag edit", result.extraUndoSnapshots, "extra-tag-update");
        }
        if (result.task && result.trackPaths) {
          if (!onAssistantRunTask) {
            throw new Error("Assistant task runner is unavailable");
          }
          try {
            await onAssistantRunTask(result.task, result.trackPaths);
            await window.api.assistantCompleteTaskActions(batchId, null);
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            try {
              await window.api.assistantCompleteTaskActions(batchId, message);
            } catch {
              // The original task error is the useful failure to report.
            }
            throw error;
          }
        } else {
          onRefreshRequest();
        }
      }
      setMessages((prev) => [
        ...prev,
        {
          role: "system",
          content: result.success
            ? "Applied action batch"
            : `Failed to apply: ${result.error}${detail ? `\n\nDetails:\n${detail}` : ""}`,
          type: result.success ? "text" : "error",
        },
      ]);
    } catch (error) {
      setMessages((prev) => [
        ...prev,
        {
          role: "system",
          content: `Error applying: ${error instanceof Error ? error.message : String(error)}`,
          type: "error",
        },
      ]);
    }
    loadPendingBatches();
    if (applyingBatchIdRef.current === batchId) {
      applyingBatchIdRef.current = null;
      setApplyProgress(null);
    }
    setApplying(false);
  };

  const handleReject = async (batchId: string) => {
    try {
      await window.api.assistantRejectActions(batchId);
      loadPendingBatches();
    } catch {
      // Ignore
    }
  };

  // Build edit handler for a given message (extracted so it's not recreated per message per render)
  const handleMsgEdit = useCallback(
    (msg: ChatMessage, index: number) => () => {
      if (msg.role === "assistant" && msg.status === "failed" && msg.userMessage) {
        setInputText(msg.userMessage);
      } else if (msg.role === "user") {
        setInputText(msg.content);
      }
      setEditingIndex(index);
      inputRef.current?.focus();
    },
    [],
  );

  // Focus input on edit
  useEffect(() => {
    if (editingIndex !== null && inputRef.current) {
      inputRef.current.focus();
    }
  }, [editingIndex]);

  const contextLabel = selectedTrackPaths.length > 0
    ? `${selectedTrackPaths.length} selected track${selectedTrackPaths.length === 1 ? "" : "s"}`
    : activeAlbumPath
      ? `Album: ${basename(activeAlbumPath)}`
      : libraryPath
        ? "Entire library"
        : "No library context";

  const handleSuggestion = (prompt: string) => {
    setInputText(prompt);
    setEditingIndex(null);
    inputRef.current?.focus();
  };

  if (!isOpen) return null;

  return (
    <aside
      aria-label="AI Assistant"
      className="fixed bottom-0 right-0 top-[38px] z-40 flex w-[420px] max-w-[calc(100vw-24px)] flex-col border-l border-border bg-white/95 text-text-primary shadow-xl shadow-black/10 backdrop-blur-xl"
    >
      {/* Header */}
      <div className="flex min-h-[56px] items-center justify-between border-b border-border/60 bg-surface-alt/40 px-4 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-accent/20 bg-accent/10 text-accent">
            <AssistantMark />
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h2 className="whitespace-nowrap text-[13px] font-semibold text-text-primary">
                AI Assistant
              </h2>
              {sessionNumber && (
                <span
                  className="max-w-[116px] truncate rounded bg-surface-alt px-1.5 py-0.5 font-mono text-[9.5px] text-text-muted"
                  title={`Session: ${sessionNumber}`}
                >
                  #{sessionNumber}
                </span>
              )}
            </div>
            <div className="mt-0.5 truncate text-[10px] text-text-muted">
              {model || "Music library copilot"}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={handleNewConversation}
            disabled={sending || applying || clearing}
            title="Start a new conversation"
            className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-[10.5px] font-medium text-text-secondary transition-all hover:bg-surface-hover hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
              <path d="M8 3v10M3 8h10" strokeLinecap="round" />
            </svg>
            New chat
          </button>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close AI Assistant"
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-text-muted transition-all hover:bg-surface-hover hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
              <path d="M12 4L4 12M4 4l8 8" strokeLinecap="round" />
            </svg>
          </button>
        </div>
      </div>

      <ScanProgressBar
        scanning={applyProgress !== null}
        progress={applyProgress ? { current: applyProgress.current, total: applyProgress.total } : null}
        label={applyProgress?.message ?? null}
      />

      {/* Messages */}
      <div className="scrollbar-thin flex-1 space-y-3 overflow-y-auto px-4 py-4">
        {messages.length === 0 && pendingBatches.length === 0 && (
          <div className="flex min-h-full flex-col items-center justify-center py-8 text-center">
            <div className="flex h-11 w-11 items-center justify-center rounded-xl border border-accent/20 bg-accent/10 text-accent shadow-sm">
              <AssistantMark size={22} />
            </div>
            <h3 className="mt-3 text-[13px] font-semibold text-text-primary">
              Ask about your music library
            </h3>
            <p className="mt-1 max-w-[290px] text-[11px] leading-relaxed text-text-secondary">
              Ask me anything about your music library. I can inspect metadata,
              research releases, and prepare changes for your approval.
            </p>
            {keyConfigured ? (
              <div className="mt-5 grid w-full grid-cols-1 gap-2">
                {SUGGESTED_PROMPTS.map((prompt) => (
                  <button
                    key={prompt}
                    type="button"
                    onClick={() => handleSuggestion(prompt)}
                    className="group flex w-full items-center justify-between rounded-lg border border-border/70 bg-white px-3 py-2 text-left text-[11px] text-text-secondary shadow-sm transition-all hover:border-accent/30 hover:bg-accent/5 hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
                  >
                    <span>{prompt}</span>
                    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" className="text-text-muted transition-transform group-hover:translate-x-0.5 group-hover:text-accent" aria-hidden="true">
                      <path d="M4 8h8M9 5l3 3-3 3" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                  </button>
                ))}
              </div>
            ) : (
              <div className="mt-5 w-full rounded-xl border border-border/70 bg-surface-alt/60 p-3 text-left">
                <div className="text-[11px] font-medium text-text-primary">
                  Connect an AI provider
                </div>
                <div className="mt-1 text-[10.5px] leading-relaxed text-text-secondary">
                  Add an API key in Settings to start a conversation.
                </div>
                <button
                  type="button"
                  onClick={onOpenSettings}
                  className="mt-3 inline-flex items-center rounded-lg bg-accent px-3 py-1.5 text-[11px] font-medium text-white shadow-sm transition-all hover:bg-accent/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 focus-visible:ring-offset-2"
                >
                  Configure AI in Settings
                </button>
              </div>
            )}
          </div>
        )}

        {messages.map((msg, i) => (
          <MessageBubble
            key={i}
            msg={msg}
            index={i}
            copiedIndex={copiedIndex}
            expanded={expandedMsgIndex === i}
            onToggleExpand={() =>
              setExpandedMsgIndex(expandedMsgIndex === i ? null : i)
            }
            onCopy={() => {
              navigator.clipboard.writeText(msg.content);
              setCopiedIndex(i);
              setTimeout(() => setCopiedIndex(null), 2000);
            }}
            onEdit={handleMsgEdit(msg, i)}
          />
        ))}

        {/* Pending action batches */}
        {pendingBatches.length > 0 && (
          <div className="space-y-2 pt-1">
            <div className="flex items-center gap-2 text-[10px] font-semibold uppercase tracking-widest text-text-muted">
              <span className="h-px flex-1 bg-border/60" />
              Review changes
              <span className="h-px flex-1 bg-border/60" />
            </div>
            {pendingBatches.map((batch) => {
              const affectedTracks = new Set(
                batch.actions.flatMap((action) => {
                  const trackPath = action.trackPath ?? action.sourcePath;
                  return trackPath ? [trackPath] : [];
                }),
              ).size;
              const riskStyles = batch.riskLevel === "high"
                ? "border-red-500/30 bg-red-500/5 text-[#ff3b30]"
                : batch.riskLevel === "medium"
                  ? "border-[#ff9f0a]/30 bg-[#ff9f0a]/5 text-[#ff9f0a]"
                  : "border-[#34c759]/30 bg-[#34c759]/5 text-[#34c759]";
              const riskLabel = `${batch.riskLevel[0].toUpperCase()}${batch.riskLevel.slice(1)} risk`;
              return (
                <div key={batch.id} className={`rounded-xl border p-3 shadow-sm ${riskStyles}`}>
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <div className="truncate text-[12px] font-semibold text-text-primary">
                        {batch.title}
                      </div>
                      <div className="mt-1 text-[10.5px] leading-relaxed text-text-secondary">
                        {batch.summary}
                      </div>
                    </div>
                    <span className="shrink-0 rounded-full border border-current/20 bg-white px-2 py-0.5 text-[9.5px] font-medium">
                      {riskLabel}
                    </span>
                  </div>
                  <div className="mt-2 text-[10px] text-text-muted">
                    {batch.actions.length} change{batch.actions.length === 1 ? "" : "s"}
                    {affectedTracks > 0
                      ? ` on ${affectedTracks} track${affectedTracks === 1 ? "" : "s"}`
                      : ""}
                  </div>

                  {batch.actions.length > 0 && (
                    <div className="scrollbar-thin mt-2 max-h-36 space-y-1 overflow-y-auto rounded-lg border border-border/60 bg-white p-2 text-[10.5px] text-text-secondary">
                      {batch.actions.slice(0, 10).map((action, actionIndex) => (
                        <div key={actionIndex} className="truncate">
                          {action.sourcePath && action.destinationPath && (
                            <span title={`${action.sourcePath} → ${action.destinationPath}`}>
                              {action.sourcePath} → {action.destinationPath}
                            </span>
                          )}
                          {action.field && (
                            <span>
                              <span className="font-medium text-text-primary">{action.field}</span>
                              {action.operation === "remove" && (
                                <span className="text-[#ff3b30]"> remove</span>
                              )}
                              {action.oldValue != null && (
                                <span className="text-[#ff3b30]"> {action.oldValue}</span>
                              )}
                              {action.newValue != null && (
                                <span className="text-[#34c759]"> → {action.newValue}</span>
                              )}
                            </span>
                          )}
                          {action.description === "move" && <span>Move to album folder</span>}
                          {action.description === "skip" && <span>Skip: {action.skipReason}</span>}
                          {action.description === "noop" && <span>Already in place</span>}
                        </div>
                      ))}
                      {batch.actions.length > 10 && (
                        <div className="text-text-muted">
                          …and {batch.actions.length - 10} more
                        </div>
                      )}
                    </div>
                  )}

                  <div className="mt-3 flex gap-2">
                    <button
                      type="button"
                      onClick={() => handleApply(batch.id)}
                      disabled={applying}
                      className="flex-1 rounded-lg bg-accent px-3 py-1.5 text-[11px] font-medium text-white shadow-sm transition-all hover:bg-accent/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:opacity-50"
                    >
                      {applying ? "Applying…" : "Apply changes"}
                    </button>
                    <button
                      type="button"
                      onClick={() => handleReject(batch.id)}
                      disabled={applying}
                      className="flex-1 rounded-lg border border-border bg-white px-3 py-1.5 text-[11px] font-medium text-text-secondary transition-all hover:bg-surface-hover hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:opacity-50"
                    >
                      Reject
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      <div className="border-t border-border/60 bg-surface-alt/40 px-4 py-3">
        <div className="mb-2 flex items-center gap-1.5 text-[10px] text-text-secondary">
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
            <circle cx="8" cy="8" r="5.5" />
            <path d="M8 5.5v5M5.5 8h5" strokeLinecap="round" />
          </svg>
          <span>{contextLabel}</span>
        </div>
        <div className="flex items-end gap-2 rounded-xl border border-border bg-white p-1.5 shadow-sm transition-all focus-within:border-accent/60 focus-within:shadow-[0_0_0_3px_rgba(0,122,255,0.12)]">
          <textarea
            ref={inputRef}
            value={inputText}
            onChange={(e) => setInputText(e.target.value)}
            onInput={(e) => resizeInput(e.currentTarget)}
            onKeyDown={handleKeyDown}
            placeholder={keyConfigured ? "Ask the assistant..." : "Configure an LLM API key in Settings"}
            disabled={sending || clearing || !keyConfigured}
            rows={1}
            aria-label="Assistant message"
            className="min-h-10 max-h-28 flex-1 resize-none overflow-y-auto bg-transparent px-2 py-2 text-[12px] leading-5 text-text-primary outline-none placeholder:text-text-muted/60 disabled:opacity-50"
            style={{ height: "40px" }}
          />
          {sending ? (
            <button
              type="button"
              onClick={handleCancel}
              aria-label="Stop response"
              className="mb-0.5 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[#ff3b30] text-white shadow-sm transition-all hover:bg-[#d92d25] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-500/40"
            >
              <span className="h-2.5 w-2.5 rounded-[2px] bg-current" aria-hidden="true" />
            </button>
          ) : (
            <button
              type="button"
              onClick={handleSend}
              disabled={!inputText.trim() || clearing || !keyConfigured}
              aria-label="Send message"
              className="mb-0.5 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent text-white shadow-sm transition-all hover:bg-accent/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:bg-surface-alt disabled:text-text-muted disabled:shadow-none"
            >
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
                <path d="M8 12V4M4.5 7.5 8 4l3.5 3.5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </button>
          )}
        </div>
        <div className="mt-1.5 px-1 text-[9.5px] text-text-muted">
          Enter to send · Shift+Enter for a new line
        </div>
      </div>
    </aside>
  );
}

function AssistantMark({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5v-3M9.5 6l2.5-1.5L14.5 6M8 14l2 3h4l2-3" />
      <circle cx="12" cy="11" r="2.5" />
    </svg>
  );
}

// ── Status icon & label helpers ───────────────────────────────────

const STATUS_CONFIG: Record<AssistantStatus, { label: string; classes: string }> = {
  sending: { label: "Sending…", classes: "border-[#ff9f0a]/20 bg-[#ff9f0a]/10 text-[#ff9f0a]" },
  thinking: { label: "Thinking…", classes: "border-accent/20 bg-accent/10 text-accent" },
  looking_up: { label: "Looking up data…", classes: "border-accent/20 bg-accent/10 text-accent" },
  applying_changes: { label: "Applying changes…", classes: "border-[#ff9f0a]/20 bg-[#ff9f0a]/10 text-[#ff9f0a]" },
  completed: { label: "Completed", classes: "border-[#34c759]/20 bg-[#34c759]/10 text-[#34c759]" },
  responded: { label: "Answered", classes: "border-accent/20 bg-accent/10 text-accent" },
  ready_for_review: { label: "Ready for review", classes: "border-[#ff9f0a]/20 bg-[#ff9f0a]/10 text-[#ff9f0a]" },
  failed: { label: "Failed", classes: "border-red-500/20 bg-red-500/10 text-[#ff3b30]" },
};

function StatusIcon({ status, pending }: { status: AssistantStatus; pending: boolean }) {
  if (status === "completed") {
    return (
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true">
        <path d="m3.5 8 3 3 6-7" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }
  if (status === "failed") {
    return (
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
        <path d="m4.5 4.5 7 7m0-7-7 7" strokeLinecap="round" />
      </svg>
    );
  }
  if (status === "responded") {
    return (
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
        <path d="M3 3.5h10v7H7l-3 2v-2H3z" strokeLinejoin="round" />
      </svg>
    );
  }
  if (status === "ready_for_review") {
    return (
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
        <rect x="3" y="3.5" width="10" height="9" rx="1.5" />
        <path d="M6 3.5V2h4v1.5M5.5 7h5M5.5 9.5h3" strokeLinecap="round" />
      </svg>
    );
  }
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      className={pending ? "animate-spin" : undefined}
      aria-hidden="true"
    >
      <circle cx="8" cy="8" r="5.5" opacity="0.3" />
      <path d="M8 2.5A5.5 5.5 0 0 1 13.5 8" strokeLinecap="round" />
    </svg>
  );
}

// ── Message bubble with status indicator, collapsible details, and actions ──

function MessageBubble({
  msg,
  index,
  copiedIndex,
  expanded,
  onToggleExpand,
  onCopy,
  onEdit,
}: {
  msg: ChatMessage;
  index: number;
  copiedIndex: number | null;
  expanded: boolean;
  onToggleExpand: () => void;
  onCopy: () => void;
  onEdit: () => void;
}) {
  const statusCfg = msg.role === "assistant" && msg.status ? STATUS_CONFIG[msg.status] : null;
  const isFailed = msg.status === "failed";
  const isPending =
    msg.status &&
    msg.status !== "ready_for_review" &&
    msg.status !== "responded" &&
    msg.status !== "completed" &&
    msg.status !== "failed";
  const isSystem = msg.role === "system";

  return (
    <div className={`group flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}>
      <div
        className={`relative rounded-xl px-3 py-2.5 text-[12px] leading-relaxed ${
          isSystem
            ? `w-full border ${msg.type === "error" ? "border-red-500/20 bg-red-500/5 text-[#ff3b30]" : "border-border/60 bg-surface-alt/60 text-text-secondary"}`
            : msg.role === "user"
              ? "max-w-[85%] bg-accent text-white shadow-sm"
              : isFailed
                ? "max-w-[92%] border border-red-500/20 bg-red-500/5 text-text-primary"
                : "max-w-[92%] border border-border/60 bg-surface-alt/60 text-text-primary shadow-sm"
        }`}
        aria-busy={isPending || undefined}
      >
        {/* Status indicator row — shown on assistant messages */}
        {statusCfg && (
          <div
            role="status"
            aria-live="polite"
            className={`mb-2 flex items-center gap-1.5 rounded-md border px-2 py-1 ${statusCfg.classes}`}
          >
            <StatusIcon status={msg.status!} pending={Boolean(isPending)} />
            <span className="text-[10px] font-medium">{statusCfg.label}</span>
            {isPending && <span className="ml-auto h-1.5 w-1.5 animate-pulse rounded-full bg-current" />}
          </div>
        )}

        {/* Message content — assistant reply text (hidden while pending with no content yet) */}
        {msg.content && (
          <div className="select-text whitespace-pre-wrap font-sans">{msg.content}</div>
        )}
        {isPending && !msg.content && (
          <div className="select-none text-[11px] italic text-text-muted">Waiting for response…</div>
        )}

        {/* Collapsible details section */}
        {msg.details && msg.details.length > 0 && (
          <div className="mt-2 border-t border-border/60 pt-1.5">
            <button
              type="button"
              onClick={onToggleExpand}
              className="flex select-none items-center gap-1 text-[10px] text-text-muted transition-colors hover:text-text-secondary focus-visible:outline-none focus-visible:text-accent"
            >
              <svg
                width="10"
                height="10"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                className={`transition-transform ${expanded ? "rotate-90" : ""}`}
                aria-hidden="true"
              >
                <path d="M6 4l4 4-4 4" />
              </svg>
              <span>
                {expanded ? "Hide details" : `${msg.details.length} step${msg.details.length !== 1 ? "s" : ""}`}
              </span>
            </button>
            {expanded && (
              <div className="mt-1.5 space-y-1">
                {msg.details.map((d, di) => (
                  <div key={di} className="flex items-start gap-1.5 text-[10px] leading-relaxed text-text-muted">
                    <span className="w-2 shrink-0 text-center text-text-muted" aria-hidden="true">{d.icon}</span>
                    <span className="min-w-0 break-words">{d.text}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Hover actions */}
        <div className="absolute -top-2 right-1 flex gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
          {/* Copy — available on user messages and completed/failed assistant messages */}
          {(msg.role === "user" || !isPending) && (
            <button
              type="button"
              onClick={onCopy}
              aria-label={copiedIndex === index ? "Message copied" : "Copy message"}
              className="inline-flex h-6 w-6 items-center justify-center rounded-md border border-border bg-white text-text-muted shadow-sm transition-colors hover:bg-surface-hover hover:text-text-primary focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
            >
              {copiedIndex === index ? (
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                  <path d="M13 4L6 12L3 8.5" />
                </svg>
              ) : (
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                  <rect x="4" y="4" width="10" height="10" rx="1.5" />
                  <path d="M12 4V2.5A1.5 1.5 0 0 0 10.5 1H3a2 2 0 0 0-2 2v7.5A1.5 1.5 0 0 0 2.5 12H4" />
                </svg>
              )}
            </button>
          )}
          {/* Edit/Retry — on user messages and failed assistant messages */}
          {(msg.role === "user" || isFailed) && (
            <button
              type="button"
              onClick={onEdit}
              aria-label={isFailed ? "Retry by editing the original prompt" : "Edit and resend"}
              className="inline-flex h-6 w-6 items-center justify-center rounded-md border border-border bg-white text-text-muted shadow-sm transition-colors hover:bg-surface-hover hover:text-text-primary focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
            >
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                <path d="M11 2L14 5L6 13H3V10L11 2Z" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
