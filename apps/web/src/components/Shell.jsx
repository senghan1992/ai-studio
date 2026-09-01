import React, { useEffect, useState } from 'react';

const APP_META = {
  deck: { name: 'AI Deck', short: 'D', color: 'var(--deck)' },
  doc: { name: 'AI Doc', short: 'W', color: 'var(--doc)' },
  grid: { name: 'AI Grid', short: 'X', color: 'var(--grid)' },
};

/**
 * Chrome shared by all three editors: title bar, ribbon slot, work area, status
 * bar. Keeping it here means the three apps look and behave like one suite
 * without each re-implementing the shell.
 */
export default function Shell({
  type, title, onTitleChange, onTitleCommit,
  dirty, saving, savedAt, onSave, onHome,
  ribbon, status, children,
  inspectorOpen, onToggleInspector,
}) {
  const app = APP_META[type] ?? APP_META.doc;
  const [draft, setDraft] = useState(title ?? '');

  useEffect(() => setDraft(title ?? ''), [title]);

  return (
    <div className="app" style={{ '--accent': app.color }}>
      <header className="titlebar">
        <button className="titlebar__brand" onClick={onHome} title="시작 화면으로">
          <span className="titlebar__logo">{app.short}</span>
          <span>{app.name}</span>
        </button>

        <div className="titlebar__title">
          <input
            className="titlebar__name"
            value={draft}
            onChange={(e) => {
              setDraft(e.target.value);
              onTitleChange?.(e.target.value);
            }}
            onBlur={() => onTitleCommit?.(draft)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') e.currentTarget.blur();
            }}
            aria-label="문서 제목"
          />
          <span className="titlebar__badge">
            {saving ? '저장 중…' : dirty ? '저장되지 않음' : '저장됨'}
          </span>
        </div>

        <div className="titlebar__actions">
          <button
            className={`tbtn tbtn--ghost${inspectorOpen ? ' tbtn--on' : ''}`}
            onClick={onToggleInspector}
            title="AI 저장 포맷 패널 표시/숨기기"
          >
            {'{ }'} 저장 포맷
          </button>
          <button className="tbtn" onClick={onSave} disabled={saving || !dirty}>
            저장
          </button>
        </div>
      </header>

      {ribbon}

      <div className="workarea">{children}</div>

      <footer className="statusbar">
        {status}
        <span className="statusbar__spacer" />
        <span>{savedAt ? `마지막 저장 ${formatTime(savedAt)}` : '아직 저장하지 않음'}</span>
      </footer>
    </div>
  );
}

function formatTime(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '—';
  return d.toLocaleTimeString('ko-KR', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}
