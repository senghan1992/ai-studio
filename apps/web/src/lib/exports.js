/** What the File tab offers per document type. */
export const EXPORT_TARGETS = {
  deck: [
    { ext: 'pptx', label: 'PowerPoint', icon: 'P', description: '.pptx — 좌표와 서식을 그대로 옮깁니다' },
    { ext: 'aimd', label: 'AI 다이제스트', icon: '🤖', description: 'AI.md — RAG에 넣는 요약 마크다운' },
  ],
  doc: [
    { ext: 'docx', label: 'Word', icon: 'W', description: '.docx — 용지·여백·문단 서식을 옮깁니다' },
    { ext: 'aimd', label: 'AI 다이제스트', icon: '🤖', description: 'AI.md — RAG에 넣는 요약 마크다운' },
  ],
  grid: [
    { ext: 'xlsx', label: 'Excel', icon: 'X', description: '.xlsx — 수식이 살아 있는 통합 문서' },
    { ext: 'csv', label: 'CSV', icon: '≡', description: '현재 시트의 값만' },
    { ext: 'aimd', label: 'AI 다이제스트', icon: '🤖', description: 'AI.md — RAG에 넣는 요약 마크다운' },
  ],
};

const enc = encodeURIComponent;

/**
 * Ask the server to convert the saved project and hand the file to the browser.
 *
 * The download goes through a blob rather than navigating, so a failed export
 * surfaces as an error message instead of replacing the editor with a JSON body.
 */
export async function downloadExport(folder, ext, title) {
  const url =
    ext === 'aimd'
      ? `/api/projects/${enc(folder)}/digest`
      : `/api/projects/${enc(folder)}/export/${enc(ext)}`;

  const res = await fetch(url);
  if (!res.ok) {
    let message = `내보내기에 실패했습니다 (${res.status})`;
    try {
      const data = await res.json();
      if (data?.error) message = data.error;
    } catch {
      /* keep the generic message */
    }
    throw new Error(message);
  }

  const blob = await res.blob();
  const name = `${sanitize(title || folder)}.${ext === 'aimd' ? 'md' : ext}`;
  const href = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = href;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Give the browser a tick to start the download before revoking.
  setTimeout(() => URL.revokeObjectURL(href), 4000);
  return name;
}

function sanitize(name) {
  return String(name).replace(/[/\\?%*:|"<>]/g, '').trim() || 'export';
}
