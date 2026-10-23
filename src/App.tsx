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

const ERR = '#ff7a70'; // lỗi/cảnh báo: san hô, tách hẳn với hổ phách (= đang đọc)

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
  // Phiên bản mới (nếu có) và cờ đang tải/cài.
  const [update, setUpdate] = useState('');
  const [updating, setUpdating] = useState(false);

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

  // Tự kiểm tra bản mới lúc mở app. Im lặng khi offline / chưa có bản phát hành nào: lỗi ở đây
  // không đáng làm phiền người đang chơi game.
  useEffect(() => {
    void import('@tauri-apps/plugin-updater')
      .then(({ check }) => check())
      .then((u) => u && setUpdate(u.version))
      .catch(() => {});
  }, []);

  // Chỉ tải + cài khi người dùng bấm — không tự cài ngầm lúc đang chơi. check() lại để lấy
  // handle Update thay vì cất object vào state.
  const doUpdate = async () => {
    setUpdating(true);
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const u = await check();
      if (u) {
        await u.downloadAndInstall();
        const { relaunch } = await import('@tauri-apps/plugin-process');
        await relaunch();
      }
    } catch (e) {
      setUpdating(false);
      setErr(`Cập nhật lỗi: ${String(e)}`);
    }
  };

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

  const setCaptions = (on: boolean, movable: boolean) => {
    setCfg({ ...cfg, captions: on, captions_movable: movable });
    void invoke('set_captions', { on, movable });
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100vh' }}>
      {update && (
        <div className="updatebar">
          <span>Có bản mới {update}</span>
          <button className="small" disabled={updating} onClick={() => void doUpdate()}>
            {updating ? 'Đang cập nhật…' : 'Cập nhật'}
          </button>
        </div>
      )}
      <div style={{ ...pad, paddingTop: 16 }}>
        <button
          className={running ? 'primary live' : 'primary'}
          style={{ width: '100%' }}
          onClick={() => void invoke<boolean>('toggle').then(setRunning)}
        >
          {running && <span className="dot" />}
          {running ? 'Dừng đọc' : 'Bắt đầu đọc'}
        </button>
      </div>

      {/* Nghe tiếng thì không chụp màn hình -> vùng quét vô nghĩa, ẩn đi cho đỡ rối. */}
      {cfg.source !== 'audio' && (
      <div style={{ ...pad, display: 'flex', gap: 8, marginTop: 8 }}>
        <button
          className="secondary"
          style={{ flex: 1 }}
          onClick={() => void invoke('pick_region', { kind: 'scan' })}
        >
          Sửa vùng quét
        </button>
        <button
          className="secondary"
          style={{ flex: 1 }}
          onClick={() => void invoke('pick_region', { kind: 'exclude' })}
        >
          Vùng loại trừ
        </button>
      </div>
      )}

      {cfg.source !== 'audio' && (
      <div style={{ ...pad, fontSize: 12, color: '#8a8f9a', marginTop: 6 }}>
        Vùng quét: {cfg.region.width}×{cfg.region.height} tại ({cfg.region.left},{cfg.region.top})
        {ex && (
          <>
            {' · loại trừ '}
            {ex.width}×{ex.height}{' '}
            <button
              className="ghost small"
              onClick={() => void invoke<Config>('clear_exclude').then(setCfg)}
            >
              xoá
            </button>
          </>
        )}
      </div>
      )}

      <div style={{ ...pad, marginTop: 10 }}>
        <label className="check">
          <input
            type="checkbox"
            checked={cfg.captions}
            onChange={(e) => setCaptions(e.target.checked, cfg.captions_movable)}
          />
          Hiện phụ đề nổi trong game
        </label>
        {cfg.captions && (
          <>
            <label className="check">
              <input
                type="checkbox"
                checked={cfg.captions_movable}
                onChange={(e) => setCaptions(cfg.captions, e.target.checked)}
              />
              Cho phép kéo để dời vị trí
            </label>
            <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 0' }}>
              Tắt mục kéo khi đã đặt xong để chuột xuyên qua vào game. Game cần chạy chế độ cửa sổ
              hoặc toàn màn hình không viền, toàn màn hình độc quyền sẽ che mất phụ đề.
            </div>
          </>
        )}
      </div>

      {/* minHeight:0 + overflow => mở khối AI ra thì phần này tự cuộn, không đẩy log ra
          ngoài cửa sổ (flex item mặc định không co xuống dưới kích thước nội dung). */}
      <div style={{ ...pad, overflowY: 'auto', minHeight: 0 }}>
        <label>Chế độ</label>
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

        <label>Lấy lời thoại từ</label>
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
        <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 6px' }}>
          {SOURCES.find((s) => s.value === cfg.source)?.ghi_chu}
          {cfg.source === 'audio' && (ready.audio ? ' · đã tải' : ' · SẼ TẢI khi chọn')}
        </div>
        {cfg.source === 'audio' && (
          <>
            <label>Nghe bằng</label>
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
            <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 6px' }}>
              {ASR_ENGINES.find((a) => a.value === cfg.asr_engine)?.ghi_chu}
              {cfg.asr_engine === 'local' && (ready.audio ? ' · đã tải' : ' · SẼ TẢI khi chọn')}
            </div>

            {cfg.asr_engine === 'speechmatics' ? (
              <>
                <label>API key Speechmatics</label>
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
                <label>Tiếng trong game</label>
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
            <label>Engine OCR</label>
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

        <hr className="sep" />

        <label>Cách dịch</label>
        <select
          value={cfg.translate_engine}
          onChange={(e) => save({ ...cfg, translate_engine: e.target.value })}
        >
          <option value="google">Google Dịch — nhanh, không cần key</option>
          <option value="llm">AI — dịch theo ngữ cảnh, cần API key</option>
        </select>

        {cfg.translate_engine === 'llm' && (
          <div style={{ borderLeft: '1px solid #2a2e36', paddingLeft: 10, marginTop: 6 }}>
            <label>Nhà cung cấp</label>
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
            <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 6px' }}>
              {preset?.free ?? 'base URL tự nhập (API kiểu OpenAI)'}
            </div>

            <label>API key</label>
            <input
              type="password"
              placeholder="dán key vào đây"
              value={cfg.llm_api_key}
              onChange={(e) => setCfg({ ...cfg, llm_api_key: e.target.value })}
              onBlur={() => save(cfg)}
            />

            <label>Model</label>
            <input
              type="text"
              value={cfg.llm_model}
              onChange={(e) => setCfg({ ...cfg, llm_model: e.target.value })}
              onBlur={() => save(cfg)}
            />

            <label>Base URL</label>
            <input
              type="text"
              value={cfg.llm_base_url}
              onChange={(e) => setCfg({ ...cfg, llm_base_url: e.target.value })}
              onBlur={() => save(cfg)}
            />

            <label>
              Ngữ cảnh &nbsp;
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
            <div style={{ fontSize: 12, color: '#8a8f9a', marginBottom: 4 }}>
              Gửi kèm câu trước giúp AI chọn xưng hô nhất quán, đổi lại tốn token và chậm hơn.
            </div>
          </div>
        )}

        <details style={{ marginTop: 12 }}>
          <summary>
            Tinh chỉnh bắt chữ
          </summary>

          <label>Nhịp quét &nbsp; {cfg.interval_ms} ms</label>
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

          <label>Độ dài dòng tối thiểu &nbsp; {cfg.min_line_len} ký tự</label>
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
            Lọc chữ rác &nbsp;
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
          <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 6px' }}>
            Nâng lên nếu còn nghe đọc chữ bịa; hạ xuống nếu câu thật bị bỏ oan.
          </div>

          <label>Chống đọc trùng &nbsp; {cfg.fuzzy_threshold.toFixed(2)}</label>
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
          <div style={{ fontSize: 12, color: '#8a8f9a', marginBottom: 4 }}>
            Hai dòng giống nhau tới mức này thì bỏ dòng sau. Hạ xuống sẽ nuốt mất câu thoại chỉ
            khác một từ — đo được 0.82 nuốt 8/8 cặp như vậy.
          </div>
        </details>

        <hr className="sep" />

        <label>Giọng đọc</label>
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
        <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 6px' }}>
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

        <label>
          Âm lượng &nbsp; {cfg.volume}%
          {cfg.volume > 100 && <span style={{ color: '#ecebe6' }}>&nbsp; · khuếch đại</span>}
        </label>
        <input
          type="range"
          min={0}
          max={300}
          step={5}
          value={cfg.volume}
          onChange={(e) => {
            const volume = Number(e.target.value);
            setCfg({ ...cfg, volume });
            void invoke('set_volume', { volume });
          }}
        />

        <label>Tốc độ &nbsp; {cfg.speed.toFixed(1)}x</label>
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
        style={{ ...pad, color: err ? ERR : '#8a8f9a', fontSize: 12, margin: '14px 0 4px' }}
      >
        {err || status || 'Ctrl+Alt+S bật/tắt · Ctrl+Alt+R đọc lại'}
      </div>
      {warn && (
        <div style={{ ...pad, color: ERR, fontSize: 12, marginBottom: 4 }}>⚠ {warn}</div>
      )}
      <div ref={logRef} className="stage">
        {log.length === 0 && (
          <div className="empty">Lời thoại sẽ hiện ở đây khi bạn bắt đầu đọc.</div>
        )}
        {log.map((l, i) => (
          <div key={i} className="cue">
            {/* bản gốc mờ ở trên: nghe sai thì nhìn là biết lỗi ở OCR hay ở khâu dịch */}
            {l.src !== l.out && <div className="src">{l.src}</div>}
            <div className="out">{l.out}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
