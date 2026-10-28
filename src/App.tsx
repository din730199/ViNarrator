import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  ASR_ENGINES,
  ASR_LANGS,
  Config,
  LLM_PRESETS,
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
    ];
    return () => {
      void Promise.all(un).then((fns) => fns.forEach((f) => f()));
    };
  }, []);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [log]);

  // Model nghe theo TỪNG thứ tiếng (Anh: zipformer, Nhật/Hàn/Trung: SenseVoice) -> hỏi lại khi
  // đổi tiếng, và mỗi lần có "status" (tải xong cũng báo qua đó).
  const lang = cfg?.asr_language;
  useEffect(() => {
    if (lang) {
      void invoke<boolean>('asr_ready', { language: lang }).then((r) =>
        setReady((x) => ({ ...x, audio: r })),
      );
    }
  }, [lang, status]);

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

  // Đổi tiếng hoặc cách nghe. Chọn bản chạy máy cho thứ tiếng chưa có model thì Rust tự tải ở nền.
  const setAsr = (doi: { engine?: string; language?: string }) =>
    void invoke<Config>('set_asr', {
      engine: doi.engine ?? cfg.asr_engine,
      key: cfg.speechmatics_key,
      language: doi.language ?? cfg.asr_language,
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
              Kéo dời vị trí, lăn chuột xem lại
            </label>
            <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 0' }}>
              Bật để kéo khung phụ đề hoặc lăn chuột xem lại 50 câu gần nhất. Tắt khi chơi để chuột
              xuyên qua vào game. Game cần chạy chế độ cửa sổ hoặc toàn màn hình không viền, toàn
              màn hình độc quyền sẽ che mất phụ đề.
            </div>
          </>
        )}
      </div>

      {/* minHeight:0 + overflow => mở khối AI ra thì phần này tự cuộn, không đẩy log ra
          ngoài cửa sổ (flex item mặc định không co xuống dưới kích thước nội dung). */}
      <div style={{ ...pad, overflowY: 'auto', minHeight: 0 }}>
        <label>Tiếng trong game</label>
        <select value={cfg.asr_language} onChange={(e) => setAsr({ language: e.target.value })}>
          {ASR_LANGS.map((l) => (
            <option key={l.value} value={l.value}>
              {l.label}
            </option>
          ))}
        </select>
        <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 6px' }}>
          Dịch sang tiếng Việt rồi đọc lên.
        </div>

        <label>Nghe bằng</label>
        <select value={cfg.asr_engine} onChange={(e) => setAsr({ engine: e.target.value })}>
          {ASR_ENGINES.map((a) => (
            <option key={a.value} value={a.value}>
              {a.label}
            </option>
          ))}
        </select>
        <div style={{ fontSize: 12, color: '#8a8f9a', margin: '2px 0 6px' }}>
          {ASR_ENGINES.find((a) => a.value === cfg.asr_engine)?.ghi_chu}
          {cfg.asr_engine === 'local' &&
            (ready.audio
              ? ' · đã tải'
              : ` · sẽ tải ${cfg.asr_language === 'en' ? '198' : '240'} MB khi bật`)}
        </div>

        {cfg.asr_engine === 'speechmatics' && (
          <>
            <label>API key Speechmatics</label>
            <input
              type="password"
              value={cfg.speechmatics_key}
              placeholder="lấy ở portal.speechmatics.com"
              onChange={(e) => setCfg({ ...cfg, speechmatics_key: e.target.value })}
              // Chỉ gửi khi rời ô: gửi theo từng phím sẽ dựng lại kết nối ở mỗi ký tự.
              onBlur={() => setAsr({})}
            />
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
            {/* bản gốc mờ ở trên: nhìn là biết lỗi do nghe sai hay do dịch sai */}
            {l.src !== l.out && <div className="src">{l.src}</div>}
            <div className="out">{l.out}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
