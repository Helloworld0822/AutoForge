import { useEffect, useRef, useState } from 'react';
import { Check, Copy, Image as ImageIcon, Loader2, Upload } from 'lucide-react';
import { listImages, uploadImage } from '../api/client';
import type { HostedImage } from '../types';

const ACCEPT = 'image/png,image/jpeg,image/gif,image/webp,image/bmp,image/svg+xml';

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function ImageHostingPage() {
  const fileRef = useRef<HTMLInputElement>(null);
  const [images, setImages] = useState<HostedImage[]>([]);
  const [loading, setLoading] = useState(true);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  const refresh = () => {
    setLoading(true);
    listImages()
      .then(setImages)
      .catch((err) => setError(err instanceof Error ? err.message : '목록을 불러오지 못했습니다.'))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    refresh();
  }, []);

  const doUpload = async (file: File) => {
    if (!file.type.startsWith('image/')) {
      setError('이미지 파일만 업로드할 수 있습니다.');
      return;
    }
    setUploading(true);
    setError(null);
    try {
      await uploadImage(file);
      refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : '업로드 실패');
    } finally {
      setUploading(false);
    }
  };

  const onDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const dropped = e.dataTransfer.files[0];
    if (dropped) doUpload(dropped);
  };

  const copyLink = (url: string) => {
    navigator.clipboard?.writeText(url).then(() => {
      setCopied(url);
      setTimeout(() => setCopied(null), 1500);
    });
  };

  return (
    <div className="mx-auto max-w-[1100px]">
      <header className="mb-6">
        <h2 className="mt-1 text-[1.75rem] font-semibold">이미지 호스팅</h2>
        <p className="mt-1.5 max-w-xl text-muted">
          이미지를 업로드하면 바로 공유·임베드 가능한 URL을 받을 수 있습니다.
        </p>
      </header>

      <div className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
        <h3 className="mb-3 flex items-center gap-2 text-[0.95rem] font-medium">
          <ImageIcon size={18} />
          이미지 업로드
        </h3>
        <div
          className={`mb-4 cursor-pointer rounded-lg border-2 border-dashed p-10 text-center transition-colors ${
            dragOver
              ? 'border-accent bg-accent-dim'
              : 'border-border hover:border-accent hover:bg-accent-dim'
          }`}
          onDragOver={(e) => {
            e.preventDefault();
            setDragOver(true);
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={onDrop}
          onClick={() => fileRef.current?.click()}
        >
          <input
            ref={fileRef}
            type="file"
            accept={ACCEPT}
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) doUpload(f);
              e.target.value = '';
            }}
          />
          {uploading ? (
            <>
              <Loader2 size={40} className="animate-spin text-accent" />
              <strong className="mt-3 block">업로드 중...</strong>
            </>
          ) : (
            <>
              <Upload size={40} className="text-muted" />
              <strong className="mt-3 block">이미지를 드래그하거나 클릭하여 업로드</strong>
              <span className="text-muted">PNG · JPG · GIF · WEBP · BMP · SVG</span>
            </>
          )}
        </div>

        {error && (
          <div className="rounded-lg border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
            {error}
          </div>
        )}
      </div>

      <section className="mb-5 rounded-lg border border-border bg-card p-5 md:p-6">
        <h3 className="mb-4 flex items-center gap-2 text-[0.95rem] font-medium">
          업로드된 이미지 ({images.length})
        </h3>
        {loading ? (
          <p className="text-muted">불러오는 중...</p>
        ) : images.length === 0 ? (
          <p className="text-muted">아직 업로드된 이미지가 없습니다.</p>
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-4">
            {images.map((img) => (
              <div
                key={img.filename}
                className="overflow-hidden rounded-lg border border-border bg-bg"
              >
                <div className="flex aspect-square items-center justify-center bg-[repeating-conic-gradient(#2a2a35_0%_25%,#23232c_0%_50%)_50%_/_16px_16px]">
                  <img
                    src={img.url}
                    alt={img.filename}
                    loading="lazy"
                    className="size-full object-contain"
                  />
                </div>
                <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                  <span className="text-xs text-muted">{formatSize(img.size)}</span>
                  <button
                    type="button"
                    className="inline-flex items-center justify-center gap-1.5 rounded-lg border border-border bg-transparent px-2.5 py-1.5 text-xs font-medium text-foreground transition-opacity hover:opacity-90"
                    onClick={() => copyLink(img.url)}
                  >
                    {copied === img.url ? (
                      <>
                        <Check size={14} /> 복사됨
                      </>
                    ) : (
                      <>
                        <Copy size={14} /> 링크 복사
                      </>
                    )}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
