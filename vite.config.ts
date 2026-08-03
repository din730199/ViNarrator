import { resolve } from 'path';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Đa trang: index (cửa sổ chính) + overlay (kéo chọn vùng quét).
export default defineConfig({
  root: resolve(__dirname),
  plugins: [react()],
  build: {
    outDir: 'dist',
    rollupOptions: {
      input: {
        index: resolve(__dirname, 'index.html'),
        overlay: resolve(__dirname, 'overlay.html'),
      },
    },
  },
  // Tauri: port cố định + strict để khớp devUrl trong tauri.conf.json.
  server: { port: 5175, strictPort: true },
  clearScreen: false,
});
