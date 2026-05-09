import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, Copy, Download, Pause, Play, Trash2 } from "lucide-react";
import { formatClock } from "../lib/time";
import type { LogEntry } from "../types/domain";

interface LiveLogViewerProps {
  logs: LogEntry[];
  paused: boolean;
  liveTail: boolean;
  onPausedChange: (paused: boolean) => void;
  onLiveTailChange: (liveTail: boolean) => void;
  onClear?: () => void;
  onExport?: () => void;
}

const BOTTOM_THRESHOLD_PX = 36;
const LOG_BOTTOM_PADDING_PX = 18;

function isNearBottom(element: HTMLElement) {
  return element.scrollHeight - element.scrollTop - element.clientHeight <= BOTTOM_THRESHOLD_PX;
}

export function LiveLogViewer({ logs, paused, liveTail, onPausedChange, onLiveTailChange, onClear, onExport }: LiveLogViewerProps) {
  const parentRef = useRef<HTMLDivElement | null>(null);
  const lastScrollTopRef = useRef(0);
  const [isAtBottom, setIsAtBottom] = useState(true);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimerRef = useRef<number | null>(null);

  const showToast = useCallback((message: string) => {
    setToast(message);
    if (toastTimerRef.current != null) window.clearTimeout(toastTimerRef.current);
    toastTimerRef.current = window.setTimeout(() => setToast(null), 1600);
  }, []);

  useEffect(() => {
    return () => {
      if (toastTimerRef.current != null) window.clearTimeout(toastTimerRef.current);
    };
  }, []);
  const rows = useMemo(() => {
    const seen = new Set<string>();
    return logs
      .filter((log) => {
        if (seen.has(log.id)) return false;
        seen.add(log.id);
        return true;
      })
      .slice(-4000);
  }, [logs]);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 32,
    getItemKey: (index) => rows[index]?.id ?? index,
    measureElement: (element) => element.getBoundingClientRect().height,
    overscan: 16
  });
  const latestLogId = rows.at(-1)?.id;

  const scrollToLatest = useCallback(() => {
    const frame = window.requestAnimationFrame(() => {
      const element = parentRef.current;
      if (!element) return;
      element.scrollTo({ top: element.scrollHeight });
      lastScrollTopRef.current = element.scrollTop;
      setIsAtBottom(true);
    });
    return () => window.cancelAnimationFrame(frame);
  }, []);

  const handleScroll = useCallback(() => {
    const element = parentRef.current;
    if (!element) return;
    const nextIsAtBottom = isNearBottom(element);
    const movedUp = element.scrollTop < lastScrollTopRef.current - 1;
    lastScrollTopRef.current = element.scrollTop;
    setIsAtBottom(nextIsAtBottom);
    if (liveTail && movedUp && !nextIsAtBottom) {
      onLiveTailChange(false);
    }
  }, [liveTail, onLiveTailChange]);

  useEffect(() => {
    const element = parentRef.current;
    if (!element) return;
    setIsAtBottom(isNearBottom(element));
  }, [latestLogId, rows.length]);

  useEffect(() => {
    if (!liveTail || paused) return undefined;
    return scrollToLatest();
  }, [latestLogId, liveTail, paused, scrollToLatest]);

  const jumpToLatest = () => {
    onLiveTailChange(true);
    scrollToLatest();
  };

  const copyVisible = async () => {
    const text = rows
      .slice(-200)
      .map((log) => `[${log.timestamp}] ${log.stream} ${log.level}: ${log.message}`)
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      showToast("Copied last 200 lines");
    } catch {
      showToast("Copy failed");
    }
  };

  const handleExport = async () => {
    if (!onExport) return;
    try {
      await onExport();
      showToast("Logs copied to clipboard");
    } catch {
      showToast("Export failed");
    }
  };

  return (
    <section className="log-viewer">
      <header className="log-toolbar">
        <div>
          <p className="eyebrow">Live logs</p>
          <strong>{rows.length.toLocaleString()} lines</strong>
        </div>
        <div className="icon-toolbar">
          <button type="button" onClick={() => onPausedChange(!paused)} title={paused ? "Resume stream" : "Pause stream"}>
            {paused ? <Play size={16} /> : <Pause size={16} />}
          </button>
          <button className={liveTail ? "active" : ""} type="button" onClick={() => onLiveTailChange(!liveTail)} title="Toggle live tail">
            Tail
          </button>
          <button type="button" onClick={copyVisible} title="Copy visible log block">
            <Copy size={16} />
          </button>
          <button type="button" onClick={handleExport} title="Export logs">
            <Download size={16} />
          </button>
          <button type="button" onClick={onClear} title="Clear local log view">
            <Trash2 size={16} />
          </button>
        </div>
      </header>
      <div ref={parentRef} className="log-scroll" onScroll={handleScroll}>
        <div style={{ height: virtualizer.getTotalSize() + LOG_BOTTOM_PADDING_PX, position: "relative" }}>
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const log = rows[virtualRow.index];
            return (
              <div
                key={virtualRow.key}
                data-index={virtualRow.index}
                ref={virtualizer.measureElement}
                className={`log-line ${log.level}`}
                style={{ transform: `translateY(${virtualRow.start}px)`, position: "absolute", top: 0, left: 0, width: "100%" }}
              >
                <span className="log-time">{formatClock(log.timestamp)}</span>
                <span className={`log-stream ${log.stream}`}>{log.stream}</span>
                <span className="log-message">{log.message}</span>
              </div>
            );
          })}
        </div>
      </div>
      {!isAtBottom ? (
        <button className="log-jump-latest" type="button" onClick={jumpToLatest}>
          <ArrowDown size={14} />
          Latest
        </button>
      ) : null}
      {toast ? (
        <div className="copy-toast" role="status" aria-live="polite">
          {toast}
        </div>
      ) : null}
    </section>
  );
}
