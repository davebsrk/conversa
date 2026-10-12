# Conversa — plan del proyecto (cliente ligero de WhatsApp Web en Rust)

> Nombre: **Conversa** — repo `davebsrk/conversa`.

## Decisiones tomadas

| Tema | Decisión |
|---|---|
| Enfoque | Shell nativa en Rust que carga WhatsApp Web en la webview del sistema (sin reimplementar el protocolo → riesgo de baneo mínimo) |
| Framework | **Tauri v2**: WebView2 en Windows, WebKitGTK en Linux. Ya trae bandeja, notificaciones, autoarranque, atajos globales, instancia única, actualizador e instaladores |
| Plataformas | Windows y Linux |
| Funciones de chat | Todas las que da WhatsApp Web (texto, grupos, multimedia, estados, encuestas…). Llamadas: solo si WA Web las soporta en esa plataforma |
| Capa nativa | Solo integración con el sistema: bandeja con contador de no leídos, notificaciones nativas, arrancar con el sistema, atajos globales |
| Segundo plano | **Modo híbrido**: al minimizar se reduce memoria y prioridad sin cortar la conexión (las notificaciones siguen siendo instantáneas) |
| Multicuenta | No en el MVP, pero la arquitectura usa perfiles aislados (un directorio de datos por cuenta) para añadirla luego |
| Objetivo de RAM | Lo más bajo posible, medido contra la app oficial, que es la referencia |
| Distribución | Windows: instalador NSIS/MSI + portable (+ winget). Linux: AppImage, .deb, .rpm, Flatpak, AUR. Auto-actualización desde GitHub Releases |
| Licencia | MIT OR Apache-2.0 |
| Repo | Git + GitHub con GitHub Actions (compilar, lint y releases en Windows y Linux) |
| Rol | El usuario dirige y decide el producto; Claude decide la técnica e implementa |

## Arquitectura

```
┌───────────────────────── proceso Rust (Tauri) ─────────────────────────┐
│  Bandeja + contador · Notificaciones nativas · Autoarranque · Atajos    │
│  Gestor de energía (visible / minimizada) · Actualizador · Ajustes      │
│                 ▲ canal IPC mínimo (solo eventos concretos)             │
│  ┌──────────────┴─────────────── webview ──────────────────────────┐    │
│  │ web.whatsapp.com  +  script inyectado:                          │    │
│  │   - intercepta `Notification` → la reenvía a Rust               │    │
│  │   - lee no leídos de `document.title` → contador en la bandeja  │    │
│  │   - enlaces externos → navegador del sistema                    │    │
│  └─────────────────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────────────┘
```

Principios:
- **Seguridad**: la página remota *no* recibe acceso a la API de Tauri. Solo hay un canal IPC estrecho con mensajes validados (notificación, contador, abrir enlace).
- **Perfil aislado**: un directorio de datos por cuenta (`profiles/<id>/`), lo que facilita la multicuenta más adelante.
- **Nada de peso extra**: sin frontend propio en el MVP. Los ajustes van en el menú de la bandeja y en un fichero de configuración TOML.

## Fases

### Fase 0 — Base y medición (antes de escribir la app)
- Crear el repo, el workspace de Cargo, la licencia, el README y `rustfmt`/`clippy` en la CI.
- **Script de medición**: suma de memoria privada y working set de *todo el árbol de procesos* (los procesos de WebView2 cuentan) y uso de CPU en reposo durante N minutos.
- Medir la app oficial de WhatsApp para Windows: arranque en frío, RAM activa, RAM minimizada y CPU en reposo. Esta es la **referencia**.

### Fase 1 — Shell mínima usable
- Ventana que carga WA Web con la sesión persistente (el QR solo se escanea una vez).
- User-Agent de un Chrome actual (WA Web rechaza navegadores "no soportados", sobre todo con WebKitGTK).
- Instancia única: abrir la app otra vez enfoca la ventana existente.
- Enlaces externos al navegador del sistema; descargas a la carpeta Descargas.
- Recordar el tamaño y la posición de la ventana.

### Fase 2 — Integración con el sistema
- Bandeja: icono con contador de no leídos; menú con Mostrar, Ocultar y Salir; cerrar la ventana la manda a la bandeja.
- Notificaciones nativas (Windows Toast / D-Bus en Linux); al hacer clic se abre ese chat.
- Arranque con el sistema (opcional, minimizada).
- Atajo global para mostrar u ocultar la ventana.

### Fase 3 — Rendimiento (el núcleo del proyecto)
- **Modo híbrido en segundo plano**:
  - Windows: `MemoryUsageTargetLevel = Low` de WebView2 al ocultar y `Normal` al mostrar; menor prioridad de proceso.
  - Linux: equivalente con la gestión de memoria de WebKitGTK.
- Ajustar los argumentos del navegador (límite de procesos de render, desactivar funciones que no se usan) y **medir cada cambio**: solo se queda lo que demuestra mejora.
- CSS inyectado opcional para quitar animaciones pesadas.
- Benchmark automatizado en CI y tabla comparativa con la app oficial en el README.

### Fase 4 — Pulido en Linux
- Wayland y X11, iconos y `.desktop`, notificaciones, portales (selector de archivos en Flatpak).
- Verificar multimedia: notas de voz, vídeo y cámara/micrófono para llamadas si WA Web las ofrece.

### Fase 5 — Distribución y actualizaciones
- `tauri bundle`: NSIS/MSI, portable, AppImage, .deb y .rpm.
- Auto-actualizador firmado (clave de firma de Tauri) que consulta GitHub Releases.
- Manifiesto Flatpak, PKGBUILD para AUR y manifiesto winget.
- Workflow de release: tag → compila → firma → publica.

### Fase 6 — Después del MVP
- Multicuenta con pestañas; las cuentas inactivas se suspenden.
- Ventana de ajustes ligera.
- (Futuro, opcional) Funciones de privacidad y personalización visual.

## Riesgos y aspectos a vigilar

| Riesgo | Impacto | Mitigación |
|---|---|---|
| El suelo de RAM lo pone WhatsApp Web + Chromium (WebView2) | El ahorro frente a la oficial puede ser moderado en uso activo | Medir desde la fase 0; el ahorro real está en segundo plano y en el arranque |
| WA Web bloquea WebKitGTK o le faltan funciones | Linux sin llamadas o con fallos | Falsear el User-Agent; documentar las limitaciones |
| Cambios de WA Web rompen el script inyectado | Fallan las notificaciones o el contador | Script mínimo, con alternativas (título de la página) y tests de humo |
| Marca registrada de Meta | Retirada del repo o de tiendas | Nombre y logo propios; aviso de "no afiliado" |
| Flathub y los envoltorios de webs | Pueden rechazar la app | Revisar sus normas antes de enviarla; el AUR y AppImage no dependen de ello |
| Windows SmartScreen sin firma de código | Aviso "editor desconocido" al instalar | Decidir si pagar un certificado (o usar SignPath para proyectos OSS) |

## Decidido después
- Nombre: **Conversa** (libre en crates.io, AUR, Arch y Flathub).
- Repo: `github.com/davebsrk/conversa`.
- Firma de código en Windows: **sin firma por ahora** (se revisa antes de la 1.0).

## Estado (2026-10-12)

| Fase | Estado |
|---|---|
| 0 — Base y medición | Hecha. Referencia: oficial minimizada 400–630 MB |
| 1 — Shell mínima | Hecha |
| 2 — Integración con el sistema | Hecha (bandeja, aviso de no leídos, notificaciones nativas, autoarranque, Ctrl+Alt+W) |
| 3 — Rendimiento | Hecha en Windows: 155 MB frente a 631 MB de la oficial (minimizadas). En Linux no hay equivalente en WebKitGTK |
| 4 — Linux | Compila en la CI; falta probarlo en un escritorio Linux real |
| 5 — Distribución | Workflow de release listo (NSIS, MSI, portable, deb, rpm, AppImage, Flatpak) y PKGBUILD. Pendiente: secretos de firma, primer release, AUR y winget (`wingetcreate new <url del instalador>`) |
| 6 — Después del MVP | Pendiente: multicuenta (los perfiles ya están aislados) |
