import { useLayoutEffect, useRef, useState } from 'react';

import LoaderGrid from './LoaderGrid';
import ShimmerLabel from './ShimmerLabel';

export interface TraceStep {
  id: string;
  /// Plain-English summary, e.g. "Listing C:/Users".
  label: string;
  /// The literal tool output the model was handed.
  detail: string;
}

/// What the assistant looked at, expandable.
///
/// This is not decoration. The whole design keeps the drive out of the
/// prompt and lets the model pull in only what it asks for — which means
/// the answer's quality depends entirely on whether it looked at the
/// right things. Showing the trace is how you check a recommendation came
/// from real data instead of being invented, and it is the reason the
/// detail rows contain the *actual* tool output rather than a paraphrase.
export default function ThinkingTrace({
  steps,
  working,
  label,
}: {
  steps: TraceStep[];
  working: boolean;
  label: string;
}) {
  const [manuallyOpen, setManuallyOpen] = useState<boolean | null>(null);
  const [openStep, setOpenStep] = useState<string | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [lineHeight, setLineHeight] = useState(0);

  // Open while it works so you can watch, closed once it settles so the
  // answer isn't buried under the trace that produced it.
  const open = manuallyOpen ?? working;

  useLayoutEffect(() => {
    if (bodyRef.current) setLineHeight(bodyRef.current.offsetHeight);
  }, [steps, open, openStep]);

  if (steps.length === 0 && !working) return null;

  const summary = working
    ? label
    : `Looked at ${steps.length} ${steps.length === 1 ? 'thing' : 'things'}`;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', marginBottom: 'var(--space-2)' }}>
      <button
        onClick={() => setManuallyOpen(!open)}
        aria-expanded={open}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          alignSelf: 'flex-start',
          background: 'transparent',
          border: 'none',
          padding: '2px 4px',
          marginLeft: -4,
          borderRadius: 'var(--radius)',
          cursor: 'pointer',
        }}
      >
        <LoaderGrid variant={working ? 'drive' : 'orbit'} />
        {working ? (
          <ShimmerLabel text={summary} />
        ) : (
          <span style={{ fontSize: 13, fontWeight: 500, color: 'var(--text-dim)' }}>{summary}</span>
        )}
        {steps.length > 0 && (
          <span
            aria-hidden
            style={{
              color: 'var(--text-dim)',
              fontSize: 10,
              transform: open ? 'rotate(180deg)' : 'none',
              transition: 'transform var(--motion-fast)',
            }}
          >
            ▾
          </span>
        )}
      </button>

      <div
        style={{
          display: 'grid',
          gridTemplateRows: open ? '1fr' : '0fr',
          opacity: open ? 1 : 0,
          transition: 'grid-template-rows var(--motion-fade), opacity var(--motion-fade)',
        }}
      >
        <div style={{ overflow: 'hidden' }}>
          <div style={{ position: 'relative', marginLeft: 5, paddingLeft: 16, marginTop: 2 }}>
            {/* The rail ties the steps together as one run. It grows with
                the list rather than being a fixed height. */}
            <span
              aria-hidden
              style={{
                position: 'absolute',
                left: 3,
                top: -4,
                width: 1,
                height: lineHeight ? lineHeight - 2 : 0,
                background: 'var(--border)',
                transition: 'height var(--motion-fade)',
              }}
            />
            <div ref={bodyRef} style={{ display: 'flex', flexDirection: 'column', gap: 2, padding: '2px 0' }}>
              {steps.map((step, i) => (
                <div key={step.id}>
                  <button
                    onClick={() => setOpenStep(openStep === step.id ? null : step.id)}
                    aria-expanded={openStep === step.id}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 8,
                      width: '100%',
                      background: openStep === step.id ? 'var(--surface-2)' : 'transparent',
                      border: 'none',
                      borderRadius: 'var(--radius)',
                      padding: '3px 6px',
                      marginLeft: -6,
                      textAlign: 'left',
                      cursor: 'pointer',
                      animation: `fade-up 300ms ease-out ${Math.min(i, 6) * 60}ms both`,
                    }}
                  >
                    <span aria-hidden style={{ color: 'var(--text-dim)', fontSize: 10, width: 8 }}>
                      {openStep === step.id ? '▾' : '▸'}
                    </span>
                    <span style={{ fontSize: 12.5, color: 'var(--text)' }}>{step.label}</span>
                  </button>

                  {openStep === step.id && (
                    <pre
                      className="mono"
                      style={{
                        margin: '2px 0 6px 8px',
                        padding: 'var(--space-2)',
                        background: 'var(--surface-2)',
                        border: '1px solid var(--border)',
                        borderRadius: 'var(--radius)',
                        fontSize: 11,
                        lineHeight: 1.5,
                        color: 'var(--text-dim)',
                        maxHeight: 260,
                        overflow: 'auto',
                        whiteSpace: 'pre',
                      }}
                    >
                      {step.detail}
                    </pre>
                  )}
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
