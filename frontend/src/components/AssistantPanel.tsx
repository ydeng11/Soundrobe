import React, { useState, useRef, useEffect, useCallback } from "react";
import type {
  AssistantEvent,
  AssistantActionBatch,
  TrackData,
  TrackUndoSnapshot,
  ExtraTagUndoSnapshot,
} from "../shared/desktop-api";

interface StatusDetail {
  icon: string;
  text: string;
}

type AssistantStatus = "sending" | "thinking" | "looking_up" | "applying_changes" | "completed" | "failed";

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
}

interface AssistantPanelProps {
  isOpen: boolean;
  onClose: () => void;
  apiKey: string;
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

export function AssistantPanel({
  isOpen,
  onClose,
  apiKey,
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
  const [pendingBatches, setPendingBatches] = useState<AssistantActionBatch[]>([]);
  const [applying, setApplying] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [sessionNumber, setSessionNumber] = useState<string | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const [expandedMsgIndex, setExpandedMsgIndex] = useState<number | null>(null);
  /** Ref for the in-flight assistant message — avoids race between setMessages batching and backend events */
  const pendingMsgRef = useRef<ChatMessage | null>(null);

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
    (updates: { status?: AssistantStatus; content?: string; detail?: StatusDetail }) => {
      const m = pendingMsgRef.current;
      if (!m) return;
      if (updates.status) m.status = updates.status;
      if (updates.content !== undefined) m.content = updates.content;
      if (updates.detail) {
        m.details = [...(m.details || []), updates.detail];
      }
      // Signal React to re-render (messages identity changes even though ref mutates)
      setMessages((prev) => [...prev]);
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
          updatePendingMsg({ status: "thinking", detail: { icon: "⚙️", text: event.message } });
          break;
        case "tool_result":
          updatePendingMsg({ status: "looking_up", detail: { icon: "📋", text: event.message } });
          break;
        case "action_batch_created":
          updatePendingMsg({ status: "completed", content: event.message, detail: { icon: "📦", text: event.message } });
          if (event.data && typeof event.data === "object" && "actionBatchId" in event.data) {
            loadPendingBatches();
          }
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "action_batch_applied":
          updatePendingMsg({ detail: { icon: "✅", text: event.message } });
          updatePendingMsg({ status: "completed" });
          loadPendingBatches();
          setSending(false);
          onRefreshRequest();
          break;
        case "action_batch_rejected":
          updatePendingMsg({ detail: { icon: "❌", text: event.message } });
          loadPendingBatches();
          break;
        case "action_batch_failed":
          updatePendingMsg({ status: "failed", detail: { icon: "⚠️", text: event.message } });
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "message":
          // Final assistant reply — set content and mark completed
          updatePendingMsg({ status: "completed", content: event.message });
          setSending(false);
          pendingMsgRef.current = null;
          refreshSessionNumber();
          break;
        case "error":
          updatePendingMsg({ status: "failed", content: event.message, detail: { icon: "⚠️", text: event.message } });
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "completed":
          if (/\b(couldn'?t complete|maximum step limit|no action was performed|malformed tool call)\b/i.test(event.message || "")) {
            updatePendingMsg({ status: "failed", content: event.message || "Incomplete.", detail: { icon: "⚠️", text: event.message || "Incomplete." } });
          } else {
            updatePendingMsg({ status: "completed", content: event.message || "Completed." });
          }
          setSending(false);
          pendingMsgRef.current = null;
          break;
        case "cancelled":
          updatePendingMsg({ status: "failed", detail: { icon: "⏹️", text: event.message || "Cancelled" } });
          setSending(false);
          pendingMsgRef.current = null;
          break;
      }
    });
    return () => unsub();
  }, [isOpen, onRefreshRequest, refreshSessionNumber, updatePendingMsg, loadPendingBatches]);

  // Load batches on mount
  useEffect(() => {
    if (isOpen) {
      loadPendingBatches();
    }
  }, [isOpen, loadPendingBatches]);

  const handleSend = async () => {
    const text = inputText.trim();
    if (!text || sending) return;

    if (text === "/clear") {
      console.log("[Assistant] /clear — resetting session");
      setInputText("");
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
      setEditingIndex(null);
      try {
        await window.api.assistantClear();
        console.log("[Assistant] Session reset complete");
        await refreshSessionNumber();
        console.log("[Assistant] Refreshed session number");
      } catch { /* runtime may not exist yet */ }
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
      await window.api.assistantSend({
        message: text,
        apiKey,
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
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleCancel = async () => {
    try {
      await window.api.assistantCancel();
      updatePendingMsg({
        status: "failed",
        detail: { icon: "⏹️", text: "Cancelled by user" },
      });
      setSending(false);
      pendingMsgRef.current = null;
    } catch {
      // Ignore
    }
  };

  const handleApply = async (batchId: string) => {
    setApplying(true);
    try {
      const result = await window.api.assistantApplyActions(batchId);
      setMessages((prev) => [
        ...prev,
        {
          role: "system",
          content: result.success
            ? `✅ Applied action batch`
            : `⚠️ Failed to apply: ${result.error}`,
          type: result.success ? "text" : "error",
        },
      ]);
      loadPendingBatches();
      if (result.success) {
        // Push undo snapshots if available
        if (result.undoSnapshots && result.undoSnapshots.length > 0) {
          onAssistantApplyUndo?.("Assistant tag edit", result.undoSnapshots, "tag-update");
        }
        if (result.extraUndoSnapshots && result.extraUndoSnapshots.length > 0) {
          onAssistantApplyUndo?.("Assistant extra tag edit", result.extraUndoSnapshots, "extra-tag-update");
        }
        if (result.task && result.trackPaths) {
          await onAssistantRunTask?.(result.task, result.trackPaths);
        } else {
          onRefreshRequest();
        }
      }
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

  if (!isOpen) return null;

  return (
    <div className="fixed inset-y-0 right-0 w-96 bg-[#1e1e2e] border-l border-[#313244] shadow-xl flex flex-col z-50">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-[#313244]">
        <div className="flex items-center gap-2 min-w-0">
          <h2 className="text-sm font-semibold text-[#cdd6f4] whitespace-nowrap">Assistant</h2>
          {sessionNumber && (
            <span
              className="text-[10px] font-mono text-[#6c7086] bg-[#313244] px-1.5 py-0.5 rounded truncate max-w-[140px]"
              title={`Session: ${sessionNumber}`}
            >
              #{sessionNumber}
            </span>
          )}
        </div>
        <button
          onClick={onClose}
          className="shrink-0 text-[#6c7086] hover:text-[#cdd6f4] transition-colors p-1"
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M12 4L4 12M4 4l8 8" />
          </svg>
        </button>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-3">
        {messages.length === 0 && (
          <div className="text-center text-[#6c7086] text-xs mt-8 space-y-2">
            <p>Ask me anything about your music library.</p>
            <p className="text-[#585b70]">Try:
              <br />"Summarize my library"
              <br />"Find tracks missing genres"
              <br />"Search MusicBrainz for this album"
              <br />"Organize this folder by extension"</p>
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
          <div className="space-y-2">
            <div className="text-xs text-[#6c7086] font-semibold">Pending Actions</div>
            {pendingBatches.map((batch) => (
              <div
                key={batch.id}
                className={`rounded-lg border px-3 py-2 text-sm ${
                  batch.riskLevel === "high"
                    ? "border-[#f38ba8] bg-[#f38ba8]/10"
                    : batch.riskLevel === "medium"
                      ? "border-[#f9e2af] bg-[#f9e2af]/10"
                      : "border-[#a6e3a1] bg-[#a6e3a1]/10"
                }`}
              >
                <div className="font-semibold text-[#cdd6f4] text-xs mb-1">
                  {batch.title}
                  <span className={`ml-2 text-[10px] px-1.5 py-0.5 rounded ${
                    batch.riskLevel === "high"
                      ? "bg-[#f38ba8]/30 text-[#f38ba8]"
                      : batch.riskLevel === "medium"
                        ? "bg-[#f9e2af]/30 text-[#f9e2af]"
                        : "bg-[#a6e3a1]/30 text-[#a6e3a1]"
                  }`}>
                    {batch.riskLevel}
                  </span>
                </div>
                <div className="text-xs text-[#a6adc8] mb-2">{batch.summary}</div>

                {/* Action details */}
                {batch.actions.length > 0 && (
                  <div className="text-[11px] text-[#6c7086] mb-2 max-h-24 overflow-y-auto">
                    {batch.actions.slice(0, 10).map((action, ai) => (
                      <div key={ai} className="truncate">
                        {action.field && (
                          <span>
                            {action.operation === "remove" ? "🗑️ " : "✏️ "}
                            <span className="text-[#cdd6f4]">{action.field}</span>
                            {action.oldValue != null && (
                              <span className="text-[#f38ba8]"> {action.oldValue}</span>
                            )}
                            {action.newValue != null && (
                              <span className="text-[#a6e3a1]"> → {action.newValue}</span>
                            )}
                          </span>
                        )}
                        {action.description === "move" && (
                          <span>📁 Move to album folder</span>
                        )}
                        {action.description === "skip" && (
                          <span>⏭️ {action.skipReason}</span>
                        )}
                        {action.description === "noop" && (
                          <span>✅ Already in place</span>
                        )}
                      </div>
                    ))}
                    {batch.actions.length > 10 && (
                      <div className="text-[#585b70]">... and {batch.actions.length - 10} more</div>
                    )}
                  </div>
                )}

                <div className="flex gap-2 mt-1">
                  <button
                    onClick={() => handleApply(batch.id)}
                    disabled={applying}
                    className="flex-1 text-xs px-2 py-1 bg-[#a6e3a1] text-[#1e1e2e] rounded hover:bg-[#94e2d5] transition-colors disabled:opacity-50"
                  >
                    {applying ? "Applying..." : "Apply"}
                  </button>
                  <button
                    onClick={() => handleReject(batch.id)}
                    disabled={applying}
                    className="flex-1 text-xs px-2 py-1 border border-[#6c7086] text-[#cdd6f4] rounded hover:bg-[#313244] transition-colors disabled:opacity-50"
                  >
                    Reject
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      <div className="border-t border-[#313244] px-4 py-3">
        <div className="flex gap-2">
          <textarea
            ref={inputRef}
            value={inputText}
            onChange={(e) => setInputText(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={apiKey ? "Ask the assistant..." : "Configure an LLM API key in Settings"}
            disabled={sending || !apiKey}
            rows={2}
            className="flex-1 bg-[#313244] text-[#cdd6f4] text-sm rounded-lg px-3 py-2 resize-none outline-none focus:ring-1 focus:ring-[#89b4fa] placeholder-[#6c7086] disabled:opacity-50"
          />
          <div className="flex flex-col gap-1">
            {sending ? (
              <button
                onClick={handleCancel}
                className="px-3 py-2 bg-[#f38ba8] text-[#1e1e2e] rounded-lg text-sm hover:bg-[#eba0ac] transition-colors"
              >
                Stop
              </button>
            ) : (
              <button
                onClick={handleSend}
                disabled={!inputText.trim() || !apiKey}
                className="px-3 py-2 bg-[#89b4fa] text-[#1e1e2e] rounded-lg text-sm hover:bg-[#b4befe] transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Send
              </button>
            )}
          </div>
        </div>
        {selectedTrackPaths.length > 0 && (
          <div className="text-[10px] text-[#6c7086] mt-1">
            Using {selectedTrackPaths.length} selected track(s) as context
          </div>
        )}
      </div>
    </div>
  );
}

// ── Status icon & label helpers ───────────────────────────────────

const STATUS_CONFIG: Record<AssistantStatus, { icon: string; label: string; color: string; bg: string }> = {
  sending: {
    icon: "⏳",
    label: "Sending…",
    color: "text-[#f9e2af]",
    bg: "bg-[#f9e2af]/10",
  },
  thinking: {
    icon: "💭",
    label: "Thinking…",
    color: "text-[#89b4fa]",
    bg: "bg-[#89b4fa]/10",
  },
  looking_up: {
    icon: "🔍",
    label: "Looking up data…",
    color: "text-[#a6e3a1]",
    bg: "bg-[#a6e3a1]/10",
  },
  applying_changes: {
    icon: "📝",
    label: "Applying changes…",
    color: "text-[#f9e2af]",
    bg: "bg-[#f9e2af]/10",
  },
  completed: {
    icon: "✅",
    label: "Completed",
    color: "text-[#a6e3a1]",
    bg: "bg-[#a6e3a1]/10",
  },
  failed: {
    icon: "❌",
    label: "Failed",
    color: "text-[#f38ba8]",
    bg: "bg-[#f38ba8]/10",
  },
};

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
  const isPending = msg.status && msg.status !== "completed" && msg.status !== "failed";

  return (
    <div className={`group flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}>
      <div
        className={`relative max-w-[85%] rounded-lg px-3 py-2 text-sm leading-relaxed ${
          msg.role === "user"
            ? "bg-[#89b4fa] text-[#1e1e2e]"
            : isFailed
              ? "bg-[#f38ba8]/10 text-[#f38ba8]"
              : isPending
                ? "bg-[#313244]/80 text-[#cdd6f4]"
                : "bg-[#313244] text-[#cdd6f4]"
        }`}
      >
        {/* Status indicator row — shown on assistant messages */}
        {statusCfg && (
          <div className={`flex items-center gap-1.5 mb-1.5 ${statusCfg.bg} rounded px-2 py-1 ${statusCfg.color}`}>
            <span className="text-[11px]">{statusCfg.icon}</span>
            <span className="text-[10px] font-medium">{statusCfg.label}</span>
            {isPending && <span className="w-1.5 h-1.5 rounded-full bg-current animate-pulse ml-auto" />}
          </div>
        )}

        {/* Message content — assistant reply text (hidden while pending with no content yet) */}
        {msg.content && (
          <div className="whitespace-pre-wrap font-sans select-text">{msg.content}</div>
        )}
        {isPending && !msg.content && (
          <div className="text-[#6c7086] text-xs italic select-none">Waiting for response…</div>
        )}

        {/* Collapsible details section */}
        {msg.details && msg.details.length > 0 && (
          <div className="mt-2 border-t border-[#313244] pt-1.5">
            <button
              onClick={onToggleExpand}
              className="flex items-center gap-1 text-[10px] text-[#6c7086] hover:text-[#a6adc8] transition-colors select-none"
            >
              <svg
                width="10"
                height="10"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                className={`transition-transform ${expanded ? "rotate-90" : ""}`}
              >
                <path d="M6 4l4 4-4 4" />
              </svg>
              <span>
                {expanded ? "Hide details" : `${msg.details.length} step${msg.details.length !== 1 ? "s" : ""}`}
              </span>
            </button>
            {expanded && (
              <div className="mt-1 space-y-0.5">
                {msg.details.map((d, di) => (
                  <div key={di} className="flex items-start gap-1 text-[10px] text-[#6c7086] leading-relaxed">
                    <span className="shrink-0">{d.icon}</span>
                    <span className="break-words min-w-0">{d.text}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Hover actions */}
        <div className="absolute -top-2 right-0 flex gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
          {/* Copy — available on user messages and completed/failed assistant messages */}
          {(msg.role === "user" || !isPending) && (
            <button
              onClick={onCopy}
              className="text-[10px] px-1.5 py-0.5 rounded bg-[#1e1e2e]/80 text-[#a6adc8] hover:text-[#cdd6f4] hover:bg-[#1e1e2e]"
              title="Copy message"
            >
              {copiedIndex === index ? (
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <path d="M13 4L6 12L3 8.5" />
                </svg>
              ) : (
                <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
                  <rect x="4" y="4" width="10" height="10" rx="1.5" />
                  <path d="M12 4V2.5A1.5 1.5 0 0 0 10.5 1H3a2 2 0 0 0-2 2v7.5A1.5 1.5 0 0 0 2.5 12H4" />
                </svg>
              )}
            </button>
          )}
          {/* Edit/Retry — on user messages and failed assistant messages */}
          {(msg.role === "user" || isFailed) && (
            <button
              onClick={onEdit}
              className="text-[10px] px-1.5 py-0.5 rounded bg-[#1e1e2e]/80 text-[#a6adc8] hover:text-[#cdd6f4] hover:bg-[#1e1e2e]"
              title={isFailed ? "Retry (edit original prompt)" : "Edit and resend"}
            >
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
                <path d="M11 2L14 5L6 13H3V10L11 2Z" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
