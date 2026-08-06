import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  ASR_ENGINES,
  ASR_LANGS,
  Config,
  ENGINES,
  LLM_PRESETS,
  MODES,
  SOURCES,
  Spoken,
  TTS_ENGINES,
  Voice,
} from './types';

const PURPLE = '#7c3aed';
const RED = '#b91c1c';
const GREEN = '#16a34a';

const pad = { padding: '0 18px' };

export default function App() {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [voices, setVoices] = useState<Voice[]>([]);
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState('');
  // Lỗi nạp phải có state RIÊNG: dùng chung `status` thì event "status" của worker ghi đè mất.
  const [err, setErr] = useState('');
  // Cảnh báo từ khâu dịch (key sai, hết quota…). Rỗng = đang chạy bình thường.
  const [warn, setWarn] = useState('');
  const [log, setLog] = useState<Spoken[]>([]);
  // Runtime của từng engine neural đã có sẵn chưa — chỉ để chọn chữ hiển thị, không chặn gì.
  const [ready, setReady] = useState<Record<string, boolean>>({});
  // Giọng VieNeu do chính model báo (worker ghi ra lúc cài) — không chép cứng ở đây,
  // model đổi danh sách thì UI tự đúng theo. Rỗng = chưa cài xong.
  const [vieneuVoices, setVieneuVoices] = useState<string[]>([]);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Lỗi command phải HIỆN LÊN — nuốt lặng thì UI kẹt ở "Đang tải…" không rõ vì sao.
    const fail = (e: unknown) => setErr(`Lỗi: ${String(e)}`);
    void invoke<Config>('get_config').then(setCfg, fail);
    void invoke<Voice[]>('list_voices').then(setVoices, fail);
    void invoke<boolean>('is_running').then(setRunning, fail);
    const hoiReady = () => {
      void invoke<boolean>('engine_ready', { engine: 'vieneu' }).then((r) =>
        setReady((x) => ({ ...x, vieneu: r })),
      );
      void invoke<string[]>('vieneu_voices').then(setVieneuVoices);
      void invoke<boolean>('asr_ready').then((r) => setReady((x) => ({ ...x, audio: r })));
    };
    hoiReady();
    const un = [
      listen<string>('status', (e) => {
        setStatus(e.payload);
        // Cài xong engine cũng báo qua "status" -> hỏi lại thay vì dò chuỗi. Event này hiếm
        // (chỉ khi đổi engine/chế độ) nên kiểm vài file là đủ rẻ.
        hoiReady();
      }),
      listen<string>('warn', (e) => setWarn(e.payload)),
      // giữ 200 dòng gần nhất — log dài không giúp gì mà tốn RAM
      listen<Spoken>('text', (e) => setLog((l) => [...l, e.payload].slice(-200))),
      listen<boolean>('running', (e) => setRunning(e.payload)), // hotkey Ctrl+Alt+S đổi trạng thái
      listen<Config>('config', (e) => setCfg(e.payload)), // overlay chọn xong vùng ở cửa sổ khác
    ];
    return () => {
      void Promise.all(un).then((fns) => fns.forEach((f) => f()));
    };
  }, []);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [log]);

  if (!cfg) return <div style={{ padding: 18 }}>{err || 'Đang tải…'}</div>;

  const ex = cfg.exclude_region;
  const preset = LLM_PRESETS.find((p) => p.base_url === cfg.llm_base_url);

  // Gõ tới đâu chỉ đổi state tại chỗ; ghi config + dựng lại translator để lúc RỜI ô
  // (onBlur) — không thì mỗi ký tự của API key là một lần ghi file + reload engine.
  const save = (c: Config) =>
    void invoke<Config>('set_translate', {
      engine: c.translate_engine,
      baseUrl: c.llm_base_url,
      model: c.llm_model,
      apiKey: c.llm_api_key,
      contextLines: c.llm_context_lines,
    }).then(setCfg);

  const saveTuning = (c: Config) =>
    void invoke<Config>('set_tuning', {
      intervalMs: c.interval_ms,
      minLineLen: c.min_line_len,
      minCleanRatio: c.min_clean_ratio,
      fuzzyThreshold: c.fuzzy_threshold,
    }).then(setCfg);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100vh' }}>
      <div style={{ ...pad, paddingTop: 16 }}>
        <button
          style={{ width: '100%', height: 46, fontSize: 15, background: running ? RED : GREEN }}
          onClick={() => void invoke<boolean>('toggle').then(setRunning)}
        >
          {running ? '⏸  DỪNG ĐỌC' : '▶  BẮT ĐẦU ĐỌC'}
        </button>
      </div>

      {/* Nghe tiếng thì không chụp màn hình -> vùng quét vô nghĩa, ẩn đi cho đỡ rối. */}
      {cfg.source !== 'audio' && (
      <div style={{ ...pad, display: 'flex', gap: 8, marginTop: 8 }}>
        <button
          style={{ flex: 1, background: PURPLE }}
          onClick={() => void invoke('pick_region', { kind: 'scan' })}
        >
          SỬA VÙNG QUÉT
        </button>
        <button
          style={{ flex: 1, background: RED }}
          onClick={() => void invoke('pick_region', { kind: 'exclude' })}
        >
          VÙNG LOẠI TRỪ
        </button>
      </div>
      )}

      {cfg.source !== 'audio' && (
      <div style={{ ...pad, fontSize: 12, color: '#9ca3af', marginTop: 6 }}>
        Vùng quét: {cfg.region.width}×{cfg.region.height} tại ({cfg.region.left},{cfg.region.top})
        {ex && (
          <>
            {' · loại trừ '}
            {ex.width}×{ex.height}{' '}
            <button
              style={{ background: '#374151', padding: '1px 6px', fontSize: 11 }}
              onClick={() => void invoke<Config>('clear_exclude').then(setCfg)}
            >
              xoá
            </button>
          </>
        )}
      </div>
      )}

      {/* minHeight:0 + overflow => mở khối AI ra thì phần này tự cuộn, không đẩy log ra
          ngoài cửa sổ (flex item mặc định không co xuống dưới kích thước nội dung). */}
      <div style={{ ...pad, overflowY: 'auto', minHeight: 0 }}>
        <label>CHẾ ĐỘ</label>
        <select
          value={cfg.mode}
          onChange={(e) => void invoke<Config>('set_mode', { mode: e.target.value }).then(setCfg)}
        >
          {MODES.map((m) => (
            <option key={m.value} value={m.value}>
              {m.label}
            </option>
          ))}
        </select>

        <label>LẤY LỜI THOẠI TỪ</label>
        <select
          value={cfg.source}
          onChange={(e) => {
            const source = e.target.value;
            // Lần đầu chọn Âm thanh sẽ tải model ở nền — báo ngay để không tưởng app treo.
            if (source === 'audio' && !ready.audio) setStatus('Đang tải model nghe…');
            void invoke<Config>('set_source', { source }).then(setCfg);
          }}
        >
          {SOURCES.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
        <div style={{ fontSize: 11, color: '#9ca3af', margin: '2px 0 6px' }}>
          {SOURCES.find((s) => s.value === cfg.source)?.ghi_chu}
          {cfg.source === 'audio' && (ready.audio ? ' · đã tải' : ' · SẼ TẢI khi chọn')}
        </div>
        {cfg.source === 'audio' && (
          <>
            <label>NGHE BẰNG</label>
            <select
              value={cfg.asr_engine}
              onChange={(e) =>
                void invoke<Config>('set_asr', {
                  engine: e.target.value,
                  key: cfg.speechmatics_key,
                  language: cfg.asr_language,
                }).then(setCfg)
              }
            >
              {ASR_ENGINES.map((a) => (
                <option key={a.value} value={a.value}>
                  {a.label}
                </option>
              ))}
            </select>
            <div style={{ fontSize: 11, color: '#9ca3af', margin: '2px 0 6px' }}>
              {ASR_ENGINES.find((a) => a.value === cfg.asr_engine)?.ghi_chu}
              {cfg.asr_engine === 'local' && (ready.audio ? ' · đã tải' : ' · SẼ TẢI khi chọn')}
            </div>

            {cfg.asr_engine === 'speechmatics' ? (
              <>
                <label>API KEY SPEECHMATICS</label>
                <input
                  type="password"
                  value={cfg.speechmatics_key}
                  placeholder="lấy ở portal.speechmatics.com"
                  onChange={(e) => setCfg({ ...cfg, speechmatics_key: e.target.value })}
                  // Chỉ gửi khi rời ô: gửi theo từng phím sẽ dựng lại kết nối ở mỗi ký tự.
                  onBlur={() =>
                    void invoke<Config>('set_asr', {
                      engine: cfg.asr_engine,
                      key: cfg.speechmatics_key,
                      language: cfg.asr_language,
                    }).then(setCfg)
                  }
                />
                <label>TIẾNG TRONG GAME</label>
                <select
                  value={cfg.asr_language}
                  onChange={(e) =>
                    void invoke<Config>('set_asr', {
                      engine: cfg.asr_engine,
                      key: cfg.speechmatics_key,
                      language: e.target.value,
                    }).then(setCfg)
                  }
                >
                  {ASR_LANGS.map((l) => (
                    <option key={l.value} value={l.value}>
                      {l.label}
                    </option>
                  ))}
                </select>
              </>
            ) : (
              // Model chạy máy chỉ biết tiếng Anh. Không cảnh báo thì người dùng chọn chế độ
              // Trung rồi ngồi chờ mãi không thấy đọc gì mà app chẳng báo lỗi gì cả.
              null
            )}
          </>
        )}

        {cfg.source !== 'audio' && (
          <>
            <label>ENGINE OCR</label>
            <select
              value={cfg.ocr_engine}
              onChange={(e) =>
                void invoke<Config>('set_engine', { engine: e.target.value }).then(setCfg)
              }
            >
              {ENGINES.map((e) => (
                <option key={e.value} value={e.value}>
                  {e.label}
                </option>
              ))}
            </select>
          </>
        )}

        <label>CÁCH DỊCH</label>
        <select
          value={cfg.translate_engine}
          onChange={(e) => save({ ...cfg, translate_engine: e.target.value })}
        >
          <option value="google">Google Dịch — nhanh, không cần key</option>
          <option value="llm">AI — dịch theo ngữ cảnh, cần API key</option>
        </select>

        {cfg.translate_engine === 'llm' && (
          <div style={{ borderLeft: `2px solid ${PURPLE}`, paddingLeft: 10, marginTop: 6 }}>
            <label>NHÀ CUNG CẤP</label>
            <select
              value={preset?.name ?? 'custom'}
              onChange={(e) => {
                const p = LLM_PRESETS.find((x) => x.name === e.target.value);
                if (p) save({ ...cfg, llm_base_url: p.base_url, llm_model: p.model });
              }}
            >
              {LLM_PRESETS.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
              {!preset && <option value="custom">Tự nhập</option>}
            </select>
            <div style={{ fontSize: 11, color: '#9ca3af', margin: '2px 0 6px' }}>
              {preset?.free ?? 'base URL tự nhập (API kiểu OpenAI)'}
            </div>

            <label>API KEY</label>
            <input
              type="password"
              placeholder="dán key vào đây"
              value={cfg.llm_api_key}
              onChange={(e) => setCfg({ ...cfg, llm_api_key: e.target.value })}
              onBlur={() => save(cfg)}
            />

            <label>MODEL</label>
            <input
              type="text"
              value={cfg.llm_model}
              onChange={(e) => setCfg({ ...cfg, llm_model: e.target.value })}
              onBlur={() => save(cfg)}
            />

            <label>BASE URL</label>
            <input
              type="text"
              value={cfg.llm_base_url}
              onChange={(e) => setCfg({ ...cfg, llm_base_url: e.target.value })}
              onBlur={() => save(cfg)}
            />

            <label>
              NGỮ CẢNH &nbsp;
              {cfg.llm_context_lines === 0
                ? 'tắt — dịch từng câu rời'
                : `${cfg.llm_context_lines} câu trước`}
            </label>
            <input
              type="range"
              min={0}
              max={5}
              value={cfg.llm_context_lines}
              onChange={(e) => setCfg({ ...cfg, llm_context_lines: Number(e.target.value) })}
              onMouseUp={() => save(cfg)}
              onKeyUp={() => save(cfg)}
            />
            <div style={{ fontSize: 11, color: '#9ca3af', marginBottom: 4 }}>
              Gửi kèm câu trước giúp AI chọn xưng hô nhất quán, đổi lại tốn token và chậm hơn.
            </div>
          </div>
        )}

        <details style={{ marginTop: 12 }}>
          <summary style={{ cursor: 'pointer', fontSize: 12, color: '#cbd5e1' }}>
            TINH CHỈNH BẮT CHỮ
          </summary>

          <label>NHỊP QUÉT &nbsp; {cfg.interval_ms} ms</label>
          <input
            type="range"
            min={300}
            max={3000}
            step={100}
            value={cfg.interval_ms}
            onChange={(e) => setCfg({ ...cfg, interval_ms: Number(e.target.value) })}
            onMouseUp={() => saveTuning(cfg)}
            onKeyUp={() => saveTuning(cfg)}
          />

          <label>ĐỘ DÀI DÒNG TỐI THIỂU &nbsp; {cfg.min_line_len} ký tự</label>
          <input
            type="range"
            min={1}
            max={12}
            value={cfg.min_line_len}
            onChange={(e) => setCfg({ ...cfg, min_line_len: Number(e.target.value) })}
            onMouseUp={() => saveTuning(cfg)}
            onKeyUp={() => saveTuning(cfg)}
          />

          <label>
            LỌC CHỮ RÁC &nbsp;
            {cfg.min_clean_ratio === 0 ? 'tắt' : cfg.min_clean_ratio.toFixed(2)}
          </label>
          <input
            type="range"
            min={0}
            max={0.95}
            step={0.05}
            value={cfg.min_clean_ratio}
            onChange={(e) => setCfg({ ...cfg, min_clean_ratio: Number(e.target.value) })}
            onMouseUp={() => saveTuning(cfg)}
            onKeyUp={() => saveTuning(cfg)}
          />
          <div style={{ fontSize: 11, color: '#9ca3af', margin: '2px 0 6px' }}>
            Nâng lên nếu còn nghe đọc chữ bịa; hạ xuống nếu câu thật bị bỏ oan.
          </div>

          <label>CHỐNG ĐỌC TRÙNG &nbsp; {cfg.fuzzy_threshold.toFixed(2)}</label>
          <input
            type="range"
            min={0.5}
            max={1}
            step={0.01}
            value={cfg.fuzzy_threshold}
            onChange={(e) => setCfg({ ...cfg, fuzzy_threshold: Number(e.target.value) })}
            onMouseUp={() => saveTuning(cfg)}
            onKeyUp={() => saveTuning(cfg)}
          />
          <div style={{ fontSize: 11, color: '#9ca3af', marginBottom: 4 }}>
            Hai dòng giống nhau tới mức này thì bỏ dòng sau. Hạ xuống sẽ nuốt mất câu thoại chỉ
            khác một từ — đo được 0.82 nuốt 8/8 cặp như vậy.
          </div>
        </details>

        <label>GIỌNG ĐỌC</label>
        <select
          value={cfg.tts_engine}
          onChange={(e) => {
            const engine = e.target.value;
            // Lần đầu chọn VieNeu sẽ cài ở nền — báo ngay để không tưởng app treo;
            // tiến độ thật (từng MB) đến sau qua event "status".
            if (engine === 'vieneu' && !ready.vieneu) setStatus('Đang cài giọng VieNeu…');
            void invoke<Config>('set_tts_engine', { engine }).then(setCfg);
          }}
        >
          {TTS_ENGINES.map((e) => (
            <option key={e.value} value={e.value}>
              {e.label}
            </option>
          ))}
        </select>
        <div style={{ fontSize: 11, color: '#9ca3af', margin: '2px 0 6px' }}>
          {TTS_ENGINES.find((e) => e.value === cfg.tts_engine)?.ghi_chu}
          {cfg.tts_engine !== 'windows' &&
            (ready[cfg.tts_engine] ? ' · đã cài' : ' · SẼ CÀI khi đọc câu đầu')}
        </div>

        <select
          value={cfg.voice_name}
          disabled={
            cfg.tts_engine === 'windows' ? voices.length === 0 : vieneuVoices.length === 0
          }
          onChange={(e) => {
            setCfg({ ...cfg, voice_name: e.target.value });
            void invoke('set_voice', { name: e.target.value });
          }}
        >
          {cfg.tts_engine === 'vieneu' ? (
            // Giọng nằm ở tham số dòng lệnh của worker -> đổi giọng dựng lại tiến trình con.
            // Đã hâm nóng sẵn ở nền nên người dùng không phải chờ ở câu kế tiếp.
            vieneuVoices.length === 0 ? (
              <option>(cài xong mới có danh sách giọng)</option>
            ) : (
              vieneuVoices.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))
            )
          ) : voices.length === 0 ? (
            <option>(chưa cài giọng tiếng Việt)</option>
          ) : (
            voices.map((v) => (
              <option key={v.id} value={v.id}>
                {v.label}
              </option>
            ))
          )}
        </select>

        <label>ÂM LƯỢNG &nbsp; {cfg.volume}%</label>
        <input
          type="range"
          min={0}
          max={100}
          value={cfg.volume}
          onChange={(e) => {
            const volume = Number(e.target.value);
            setCfg({ ...cfg, volume });
            void invoke('set_volume', { volume });
          }}
        />

        <label>TỐC ĐỘ &nbsp; {cfg.speed.toFixed(1)}x</label>
        <input
          type="range"
          min={0.5}
          max={2}
          step={0.1}
          value={cfg.speed}
          onChange={(e) => {
            const speed = Number(e.target.value);
            setCfg({ ...cfg, speed });
            void invoke('set_speed', { speed });
          }}
        />
      </div>

      <div
        style={{ ...pad, color: err ? '#f87171' : '#9ca3af', fontSize: 12, margin: '14px 0 4px' }}
      >
        {err || status || 'Ctrl+Alt+S bật/tắt · Ctrl+Alt+R đọc lại'}
      </div>
      {warn && (
        <div style={{ ...pad, color: '#fbbf24', fontSize: 12, marginBottom: 4 }}>⚠ {warn}</div>
      )}
      <div
        ref={logRef}
        style={{
          flex: 1,
          minHeight: 90,
          overflowY: 'auto',
          margin: '0 18px 16px',
          padding: 8,
          background: '#111',
          borderRadius: 6,
          whiteSpace: 'pre-wrap',
        }}
      >
        {log.map((l, i) => (
          <div key={i} style={{ marginBottom: 6 }}>
            {/* bản gốc mờ ở trên: nghe sai thì nhìn là biết lỗi ở OCR hay ở khâu dịch */}
            {l.src !== l.out && (
              <div style={{ color: '#6b7280', fontSize: 11 }}>{l.src}</div>
            )}
            <div>{l.out}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
