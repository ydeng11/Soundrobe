import React, { useState, useRef, useEffect, useCallback } from "react";
import type {
  AssistantEvent,
  AssistantActionBatch,
  TrackData,
  TrackUndoSnapshot,
  ExtraTagUndoSnapshot,
} from "../../electron/preload";

interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
  type?: "text" | "tool_running" | "tool_result" | "action_batch" | "error";
  batch?: AssistantActionBatch;
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
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);

  // Scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // Focus input when panel opens
  useEffect(() => {
    if (isOpen && inputRef.current) {
      inputRef.current.focus();
    }
  }, [isOpen]);

  // Listen for assistant events
  useEffect(() => {
    if (!isOpen) return;
    const unsub = window.api.onAssistantEvent((event: AssistantEvent) => {
      switch (event.type) {
        case "message":
          setMessages((prev) => [
            ...prev,
            { role: "assistant", content: event.message, type: "text" },
          ]);
          setSending(false);
          break;
        case "tool_running":
          setMessages((prev) => [
            ...prev,
            { role: "system", content: event.message, type: "tool_running" },
          ]);
          break;
        case "tool_result":
          setMessages((prev) => [
            ...prev,
            { role: "system", content: event.message, type: "tool_result" },
          ]);
          break;
        case "action_batch_created":
          if (event.data && typeof event.data === "object" && "actionBatchId" in event.data) {
            // Batch will be fetched separately
            loadPendingBatches();
          }
          setSending(false);
          break;
        case "action_batch_applied":
          setMessages((prev) => [
            ...prev,
            { role: "system", content: `✅ ${event.message}`, type: "text" },
          ]);
          loadPendingBatches();
          onRefreshRequest();
          break;
        case "action_batch_rejected":
          setMessages((prev) => [
            ...prev,
            { role: "system", content: `❌ ${event.message}`, type: "text" },
          ]);
          loadPendingBatches();
          break;
        case "action_batch_failed":
          setMessages((prev) => [
            ...prev,
            { role: "system", content: `⚠️ ${event.message}`, type: "error" },
          ]);
          break;
        case "error":
          setMessages((prev) => [
            ...prev,
            { role: "system", content: `Error: ${event.message}`, type: "error" },
          ]);
          setSending(false);
          break;
        case "completed":
        case "cancelled":
          setSending(false);
          break;
      }
    });
    return () => unsub();
  }, [isOpen, onRefreshRequest]);

  const loadPendingBatches = useCallback(async () => {
    try {
      const batches = await window.api.assistantGetBatches();
      setPendingBatches(batches);
    } catch {
      // Ignore
    }
  }, []);

  // Load batches on mount
  useEffect(() => {
    if (isOpen) {
      loadPendingBatches();
    }
  }, [isOpen, loadPendingBatches]);

  const handleSend = async () => {
    const text = inputText.trim();
    if (!text || sending) return;

    setInputText("");
    setEditingIndex(null);
    setSending(true);
    setMessages((prev) => [
      ...prev,
      { role: "user", content: text, type: "text" },
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
    } catch (error) {
      setMessages((prev) => [
        ...prev,
        {
          role: "system",
          content: `Failed to send: ${error instanceof Error ? error.message : String(error)}`,
          type: "error",
        },
      ]);
      setSending(false);
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
      setSending(false);
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
        <h2 className="text-sm font-semibold text-[#cdd6f4]">Assistant</h2>
        <button
          onClick={onClose}
          className="text-[#6c7086] hover:text-[#cdd6f4] transition-colors p-1"
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
            onCopy={() => {
              navigator.clipboard.writeText(msg.content);
              setCopiedIndex(i);
              setTimeout(() => setCopiedIndex(null), 2000);
            }}
            onEdit={() => {
              setInputText(msg.content);
              setEditingIndex(i);
              inputRef.current?.focus();
            }}
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

// ── Message bubble with copy/edit actions ─────────────────────────

function MessageBubble({
  msg,
  index,
  copiedIndex,
  onCopy,
  onEdit,
}: {
  msg: ChatMessage;
  index: number;
  copiedIndex: number | null;
  onCopy: () => void;
  onEdit: () => void;
}) {
  return (
    <div className={`group flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}>
      <div
        className={`relative max-w-[85%] rounded-lg px-3 py-2 text-sm leading-relaxed ${
          msg.role === "user"
            ? "bg-[#89b4fa] text-[#1e1e2e]"
            : msg.type === "error"
              ? "bg-[#f38ba8]/20 text-[#f38ba8]"
              : msg.type === "tool_running"
                ? "bg-[#f9e2af]/10 text-[#f9e2af] text-xs italic"
                : msg.type === "tool_result"
                  ? "bg-[#a6e3a1]/10 text-[#a6e3a1] text-xs"
                  : "bg-[#313244] text-[#cdd6f4]"
        }`}
      >
        <div className="whitespace-pre-wrap font-sans select-text">{msg.content}</div>

        {/* Hover actions — only for user messages */}
        {msg.role === "user" && (
          <div className="absolute -top-2 right-0 flex gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
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
            <button
              onClick={onEdit}
              className="text-[10px] px-1.5 py-0.5 rounded bg-[#1e1e2e]/80 text-[#a6adc8] hover:text-[#cdd6f4] hover:bg-[#1e1e2e]"
              title="Edit and resend"
            >
              <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
                <path d="M11 2L14 5L6 13H3V10L11 2Z" />
              </svg>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
