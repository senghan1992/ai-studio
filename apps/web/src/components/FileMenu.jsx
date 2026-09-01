import React, { useState } from 'react';
import { api } from '../api.js';
import { Group, Btn, Dialog, Field } from './ui.jsx';
import { EXPORT_TARGETS, downloadExport } from '../lib/exports.js';

/**
 * The ribbon's File tab.
 *
 * Office puts new / open / save / save-as / export / print / close here, and
 * people look for them here. Rendering it as a ribbon tab rather than a takeover
 * screen keeps one interaction model for the whole window.
 */
export default function FileMenu({
  project, dirty, onSave, onHome, onNewProject, onPrint, notify,
}) {
  const [copyDialog, setCopyDialog] = useState(null);
  const [busy, setBusy] = useState(false);
  const [exporting, setExporting] = useState(null);

  const type = project.type;
  const folder = project.folder;
  const targets = EXPORT_TARGETS[type] ?? [];

  const saveCopy = async () => {
    const title = copyDialog.title.trim();
    if (!title) return;
    setBusy(true);
    try {
      const created = await api.createProject(type, title, false);
      const key = { deck: 'slides', doc: 'sections', grid: 'sheets' }[type];
      await api.saveProject(created.folder, { manifest: { title }, [key]: project[key] });
      setCopyDialog(null);
      notify(`사본을 만들었습니다: ${created.folder}`);
    } catch (e) {
      notify(e.message);
    } finally {
      setBusy(false);
    }
  };

  const runExport = async (target) => {
    setExporting(target.ext);
    try {
      await downloadExport(folder, target.ext, project.manifest?.title);
      notify(`${target.label} 파일을 내려받았습니다`);
    } catch (e) {
      notify(e.message);
    } finally {
      setExporting(null);
    }
  };

  return (
    <>
      <Group label="문서">
        <Btn icon="🞤" label="새로 만들기" onClick={() => onNewProject?.()} />
        <Btn icon="📂" label="열기" title="시작 화면으로 돌아갑니다" onClick={onHome} />
        <Btn icon="💾" label="저장" onClick={onSave} disabled={!dirty} title="Ctrl+S" />
        <Btn
          icon="⧉"
          label="사본 만들기"
          onClick={() => setCopyDialog({ title: `${project.manifest?.title ?? '문서'} 사본` })}
        />
      </Group>

      <Group label="내보내기">
        {targets.map((target) => (
          <Btn
            key={target.ext}
            icon={target.icon}
            label={exporting === target.ext ? '내보내는 중…' : target.label}
            title={`${target.description} — 서버에서 변환해 내려받습니다`}
            disabled={!!exporting}
            onClick={() => runExport(target)}
          />
        ))}
      </Group>

      {onPrint && (
        <Group label="인쇄">
          <Btn icon="🖨" label="인쇄" title="Ctrl+P" onClick={onPrint} />
        </Group>
      )}

      <Group label="정보">
        <div style={{ fontSize: 11.5, color: 'var(--ink-2)', lineHeight: 1.7, padding: '2px 6px', maxWidth: 320 }}>
          <div>
            폴더 <code>{folder}</code>
          </div>
          <div>
            만든 날짜 {formatDate(project.manifest?.created)}
          </div>
          <div>
            형식 <code>{project.manifest?.format}</code> v{project.manifest?.formatVersion}
          </div>
        </div>
      </Group>

      {copyDialog && (
        <Dialog
          title="사본 만들기"
          confirmLabel="만들기"
          busy={busy}
          onCancel={() => setCopyDialog(null)}
          onConfirm={saveCopy}
        >
          <p>현재 내용을 새 프로젝트 폴더로 복사합니다. 원본은 그대로 남습니다.</p>
          <Field label="새 문서 제목">
            <input value={copyDialog.title} onChange={(e) => setCopyDialog({ title: e.target.value })} />
          </Field>
        </Dialog>
      )}
    </>
  );
}

function formatDate(iso) {
  if (!iso) return '—';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '—' : d.toLocaleDateString('ko-KR', { dateStyle: 'medium' });
}
