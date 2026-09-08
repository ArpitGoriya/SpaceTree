import { useCallback, useEffect, useRef, useState } from 'react';

import { api, onAiDelta, onAiDone, onAiError, onAiToolCall } from '../../api';
import Markdown from './Markdown';
import ThinkingTrace, { type TraceStep } from './ThinkingTrace';

export interface ChatTurn {
  role: 'user' | 'assistant';
  content: string;
  /// What the assistant inspected to produce this answer.
  steps?: TraceStep[];
  error?: boolean;
}

const SUGGESTIONS = [
  'What can I safely delete?',
  'What is using the most space?',
  'Find my biggest files',
  'Are there caches or build folders I can clear?',
];

export default function AssistantPanel({
  configured,
  onOpenSettings,
  onPathClick,
  onClose,
  fullscreen,
  onToggleFullscreen,
}: {
  configured: boolean;
  onOpenSettings: () => void;
  onPathClick: (path: string) => void;
  onClose: () => void;
  fullscreen: boolean;
  onToggleFullscreen: () => void;
}) {
  const [turns, setTurns] = useState<ChatTurn[]>([]);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [steps, setSteps] = useState<TraceStep[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  // Held in a ref as well as state: the event handlers below are
  // registered once and would otherwise close over the first render's
  // empty draft.
  const draftRef = useRef('');

  // Follow the answer as it streams, but only while already at the
  // bottom — yanking the view back while someone reads earlier output is
  // the thing every chat UI gets wrong.
  const stickToBottom = useRef(true);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !stickToBottom.current) return;
    el.scrollTop = el.scrollHeight;
  }, [turns, steps]);

  useEffect(() => {
    const unlisteners = [
      onAiDelta((delta) => {
        draftRef.current += delta;
        setTurns((prev) => replaceDraft(prev, draftRef.current));
      }),
      onAiToolCall((step) => {
        setSteps((prev) => [...prev, step]);
      }),
      onAiDone(() => {
        setBusy(false);
        setSteps((collected) => {
          setTurns((prev) => finishDraft(prev, draftRef.current, collected));
          return [];
        });
        draftRef.current = '';
      }),
      onAiError((message) => {
        setBusy(false);
        draftRef.current = '';
        setSteps([]);
        setTurns((prev) => [
          ...prev.filter((t) => !isDraft(t)),
          { role: 'assistant', content: message, error: true },
        ]);
      }),
    ];
    return () => {
      for (const u of unlisteners) u.then((f) => f());
    };
  }, []);

  const send = useCallback(
    async (question: string) => {
      const text = question.trim();
      if (!text || busy) return;
      setInput('');
      draftRef.current = '';
      setSteps([]);
      stickToBottom.current = true;

      // The history sent to the model is prose only — tool results are
      // deliberately not replayed, since the model can just call the tool
      // again and they are by far the bulkiest thing in a transcript.
      const history = turns
        .filter((t) => !t.error)
        .map((t) => ({ role: t.role, content: t.content }));

      setTurns((prev) => [
        ...prev,
        { role: 'user', content: text },
        { role: 'assistant', content: '' },
      ]);
      setBusy(true);
      try {
        await api.aiAsk(text, history);
      } catch {
        // The backend also emits `ai_error`, which renders the message in
        // the transcript; nothing to add here.
      }
    },
    [busy, turns],
  );

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        minWidth: 0,
        background: 'var(--bg)',
        borderLeft: fullscreen ? undefined : '1px solid var(--border)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-2)',
          height: 30,
          padding: '0 var(--space-2) 0 var(--space-3)',
          borderBottom: '1px solid var(--border)',
          flexShrink: 0,
        }}
      >
        <span className="label" style={{ flex: 1 }}>
          Assistant
        </span>
        {turns.length > 0 && (
          <button
            onClick={() => {
              setTurns([]);
              setSteps([]);
              draftRef.current = '';
            }}
            title="Clear this conversation"
            style={{ padding: '2px var(--space-2)' }}
          >
            Clear
          </button>
        )}
        <button onClick={onOpenSettings} title="Assistant settings" style={{ padding: '2px var(--space-2)' }}>
          Settings
        </button>
        <button
          onClick={onToggleFullscreen}
          title={fullscreen ? 'Shrink to the side panel' : 'Expand to fill the window'}
          style={{ padding: '2px var(--space-2)' }}
        >
          {fullscreen ? '⤡' : '⤢'}
        </button>
        <button onClick={onClose} title="Close the assistant" style={{ padding: '2px var(--space-2)' }}>
          ✕
        </button>
      </div>

      <div
        ref={scrollRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        style={{ flex: 1, overflowY: 'auto', padding: 'var(--space-3)', minHeight: 0 }}
      >
        {!configured && (
          <div
            className="panel"
            style={{ padding: 'var(--space-3)', marginBottom: 'var(--space-3)' }}
          >
            <div style={{ fontWeight: 500, marginBottom: 4 }}>Not connected yet</div>
            <div className="dim" style={{ fontSize: 'var(--text-secondary)', marginBottom: 'var(--space-3)' }}>
              The assistant runs on OpenRouter. Add an API key and pick a model — there are free
              ones — and it can read this scan and tell you what is worth clearing.
            </div>
            <button className="primary" onClick={onOpenSettings}>
              Open Settings
            </button>
          </div>
        )}

        {configured && turns.length === 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-2)' }}>
            <div className="dim" style={{ fontSize: 'var(--text-secondary)' }}>
              Ask about this scan. The assistant can look inside any folder, but only reads — it
              never deletes anything.
            </div>
            {SUGGESTIONS.map((s) => (
              <button
                key={s}
                onClick={() => void send(s)}
                style={{ textAlign: 'left', padding: 'var(--space-2) var(--space-3)' }}
              >
                {s}
              </button>
            ))}
          </div>
        )}

        {turns.map((turn, i) => {
          const last = i === turns.length - 1;
          return (
            <div key={i} style={{ marginBottom: 'var(--space-4)' }}>
              {turn.role === 'user' ? (
                <div
                  style={{
                    background: 'var(--surface-2)',
                    border: '1px solid var(--border)',
                    borderRadius: 'var(--radius)',
                    padding: 'var(--space-2) var(--space-3)',
                    marginLeft: 'auto',
                    maxWidth: '85%',
                    width: 'fit-content',
                    whiteSpace: 'pre-wrap',
                  }}
                >
                  {turn.content}
                </div>
              ) : (
                <div style={{ fontSize: 'var(--text-body)' }}>
                  <ThinkingTrace
                    steps={last && busy ? steps : (turn.steps ?? [])}
                    working={last && busy}
                    label={steps.length === 0 ? 'Reading the scan' : 'Working'}
                  />
                  {turn.error ? (
                    <div style={{ color: 'var(--danger)' }}>{turn.content}</div>
                  ) : (
                    <>
                      <Markdown text={turn.content} onPathClick={onPathClick} />
                      {last && busy && turn.content.length > 0 && (
                        <span
                          aria-hidden
                          className="ai-caret"
                          style={{
                            display: 'inline-block',
                            width: 2,
                            height: 13,
                            background: 'var(--text)',
                            verticalAlign: 'text-bottom',
                            marginLeft: 2,
                            animation: 'caret-blink 1s step-end infinite',
                          }}
                        />
                      )}
                    </>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      <div
        style={{
          borderTop: '1px solid var(--border)',
          padding: 'var(--space-2) var(--space-3)',
          flexShrink: 0,
          display: 'flex',
          gap: 'var(--space-2)',
          alignItems: 'flex-end',
        }}
      >
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends; Shift+Enter is a newline, as everywhere else.
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              void send(input);
            }
          }}
          placeholder={configured ? 'Ask about this scan…' : 'Add an API key in Settings first'}
          disabled={!configured}
          rows={1}
          style={{
            flex: 1,
            resize: 'none',
            maxHeight: 120,
            minHeight: 30,
            fontFamily: 'inherit',
            fontSize: 'inherit',
            color: 'var(--text)',
            background: 'var(--surface)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius)',
            padding: 'var(--space-2) var(--space-3)',
          }}
        />
        {busy ? (
          <button onClick={() => void api.aiCancel()} title="Stop generating">
            Stop
          </button>
        ) : (
          <button className="primary" disabled={!configured || !input.trim()} onClick={() => void send(input)}>
            Send
          </button>
        )}
      </div>
    </div>
  );
}

/// The in-flight assistant turn is the trailing empty/partial one.
function isDraft(turn: ChatTurn): boolean {
  return turn.role === 'assistant' && !turn.error && turn.steps === undefined;
}

function replaceDraft(turns: ChatTurn[], content: string): ChatTurn[] {
  const next = [...turns];
  const last = next[next.length - 1];
  if (last && isDraft(last)) {
    next[next.length - 1] = { ...last, content };
  }
  return next;
}

/// Freeze the streamed answer, attaching the trace that produced it so it
/// stays inspectable after the fact.
function finishDraft(turns: ChatTurn[], content: string, steps: TraceStep[]): ChatTurn[] {
  const next = [...turns];
  const last = next[next.length - 1];
  if (last && isDraft(last)) {
    next[next.length - 1] = { ...last, content: content || last.content, steps };
  }
  return next;
}
