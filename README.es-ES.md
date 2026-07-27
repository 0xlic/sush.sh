# sush

> SSH y SFTP, finalmente viviendo bajo el mismo techo.

`sush` es una herramienta diminuta, rápida y nativa de terminal para gestionar conexiones SSH y transferencias de archivos SFTP, sin tener que despegar las manos del teclado.

**[中文文档 →](docs/README.zh.md)**

---

## El problema

Te conectas por SSH a un servidor. Luego te das cuenta de que necesitas un archivo. Entonces:

1. Abres una nueva pestaña de la terminal.
2. Te peleas con `sftp user@host`.
3. Olvidas la ruta que estabas mirando hace un momento.
4. Te rindes y usas `scp` de memoria.
5. Te equivocas en la ruta de todos modos.

`sush` soluciona esto tratando SSH y SFTP como dos vistas de la misma sesión: presiona `Ctrl-\` para alternar entre ellas. Eso es todo.

---

## Demo

```
┌─ sush ─────────────────────────────────────────────┐
│                                                     │
│  > prod█                                            │
│                                                     │
│  ┌───────────────────────────────────────────────┐  │
│  │ ● prod-web-01   192.168.1.10          web    │  │
│  │   prod-db-01    192.168.1.20           db    │  │
│  │   prod-cache    192.168.1.30        cache    │  │
│  └───────────────────────────────────────────────┘  │
│                                                     │
│  /:search  Enter:SSH  s:SFTP  q:quit               │
└─────────────────────────────────────────────────────┘
```

Escribe para realizar una búsqueda difusa (fuzzy-search). Presiona Enter para conectar. Presiona `Ctrl-\` en cualquier momento para cambiar al navegador SFTP (tu sesión de SSH permanece activa). Presiona `Ctrl-\` nuevamente para volver.

---

## Características

**SSH sin fricciones**
- Lee `~/.ssh/config` automáticamente al iniciar; tus hosts ya estarán allí.
- Búsqueda difusa por nombre de host, IP, usuario, etiquetas y descripción.
- Las etiquetas de tipo ruta crean una barra lateral de carpetas virtuales dentro de la vista principal.
- Emulador de terminal embebido: `vim`, `tmux`, `htop` funcionan correctamente.
- Navegación de historial con Scrollback, PageUp/PageDown y copia mediante selección con ratón en pantallas de terminal normales.

**Cambio fluido SSH ↔ SFTP**
- `Ctrl-\` alterna entre la shell de SSH y el navegador SFTP.
- SSH y SFTP comparten una única conexión TCP; no hay re-autenticación.
- El contexto del directorio de trabajo se preserva.

**SFTP que no es un dolor de cabeza**
- Las terminales anchas muestran los paneles local y remoto lado a lado; las estrechas muestran solo el panel activo.
- `Tab` para cambiar el foco entre los paneles local y remoto sin perder la selección de cada lado.
- `d` para descargar, `u` para subir, con un indicador de transferencia global en la parte inferior derecha.
- Las transferencias de directorios mantienen el directorio seleccionado y muestran el progreso agregado `N/M`.
- `e` para abrir un archivo remoto en la aplicación GUI predeterminada de tu sistema y subida automática al guardar.
- `Enter` para entrar en directorios, `Backspace`/`Left` para subir de nivel, `/` para filtrar la lista actual y `g` para saltar a una ruta.
- Una cola FIFO limitada a la conexión mantiene las transferencias ejecutándose mientras te mueves entre Principal, SSH y SFTP.

**Gestión de reenvío de puertos (Port forwarding)**
- Presiona `p` desde la vista principal para abrir el gestor de reenvíos: lista de hosts a la izquierda, reglas a la derecha.
- Reglas de reenvío local, remoto y dinámico (SOCKS5), almacenadas por host.
- ProxyJump de un solo salto: las reglas pueden tunelar a través de un bastión antes del host de destino.
- Un demonio en segundo plano mantiene abiertas las conexiones para que los reenvíos sobrevivan después de salir de sush.
- La columna de estado muestra `Running`, `Reconnecting`, `Error`, etc., en tiempo real.

**Compatibilidad con bastiones PuTTY**
- Presiona `,` desde la vista principal para abrir los Ajustes.
- El lanzador de compatibilidad con PuTTY está desactivado por defecto y solo puede habilitarse desde Ajustes.
- En Windows, Ajustes instala una ruta shim de `putty.exe` gestionada por sush para clientes de bastión que esperan PuTTY.
- Las llamadas de bastión como `putty.exe -ssh -l user -P 2222 host` abren una nueva terminal local y entran directamente en la sesión SSH.
- `-pw` se utiliza solo como contraseña en memoria para el lanzamiento actual; no se guarda en la configuración, llavero, historial o texto de estado.

**Ágil**
- Inicia en menos de 200ms.
- La búsqueda responde en menos de 50ms.
- Memoria en reposo inferior a 30MB.

---

## Instalación

### Script de instalación (macOS / Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/0xlic/sush.sh/main/scripts/install.sh | sh
```

El instalador detecta macOS o Linux junto con la arquitectura de CPU, descarga el activo de GitHub Release correspondiente, lo verifica contra `sha256.sum` e instala `sush` en `$HOME/.local/bin`.

```sh
# Instalar una versión específica
curl -fsSL https://raw.githubusercontent.com/0xlic/sush.sh/main/scripts/install.sh | sh -s -- v1.3.0

# Instalar en un directorio personalizado
curl -fsSL https://raw.githubusercontent.com/0xlic/sush.sh/main/scripts/install.sh | SUSH_INSTALL_DIR=/usr/local/bin sh
```

Ejecuta el mismo comando nuevamente para actualizar a la última versión estable.

### Borrador de fórmula de Homebrew

Un borrador de fórmula de Homebrew está disponible en `packaging/homebrew/sush.rb`. Utiliza los activos de lanzamiento de macOS y los valores sha256 fijados de la versión estable actual.

```sh
brew install --formula packaging/homebrew/sush.rb
```

### Desde binario

Descarga el último lanzamiento para tu plataforma desde [GitHub Releases](https://github.com/0xlic/sush.sh/releases):

| Plataforma      | Archivo                                     |
|-----------------|---------------------------------------------|
| macOS (Apple)   | `sush-aarch64-apple-darwin.tar.xz`         |
| macOS (Intel)   | `sush-x86_64-apple-darwin.tar.xz`          |
| Linux arm64     | `sush-aarch64-unknown-linux-gnu.tar.xz`     |
| Linux x86_64    | `sush-x86_64-unknown-linux-gnu.tar.xz`      |
| Windows x86     | `sush-i686-pc-windows-msvc.zip`            |
| Windows x86_64  | `sush-x86_64-pc-windows-msvc.zip`           |

```sh
# macOS / Linux
tar -xf sush-*.tar.xz
chmod +x sush
mv sush /usr/local/bin/sush
sush
```

Para verificar una descarga manual, descarga `sha256.sum` del mismo lanzamiento y ejecuta:

```sh
shasum -a 256 -c sha256.sum --ignore-missing
```

En Linux, también se admite `sha256sum -c sha256.sum --ignore-missing`.

### Desde el código fuente

```sh
git clone https://github.com/0xlic/sush.sh
cd sush.sh
cargo build --release
./target/release/sush
```

Requiere Rust 1.95+. Sin otras dependencias.

---

## Inicio rápido

```sh
sush
```

En el primer lanzamiento, `sush` preguntará si desea importar desde `~/.ssh/config`. También puede presionar `n` para agregar hosts manualmente, o `i` para importar en cualquier momento.

**Navegación**
| Tecla | Acción |
|-------|--------|
| `/` o simplemente escribir | Enfocar búsqueda |
| `↑` / `↓` | Moverse por la lista de hosts |
| `Enter` | Conectar vía SSH |
| `s` | Abrir navegador SFTP |
| `n` | Nuevo host |
| `e` | Editar host seleccionado |
| `d` | Eliminar host seleccionado |
| `i` | Importar desde `~/.ssh/config` |
| `f` | Alternar barra lateral de carpetas |
| `p` | Abrir gestor de reenvío de puertos |
| `,` | Abrir Ajustes |
| `j` | Saltar a carpeta (cuando las carpetas están enfocadas) |
| `q` | Salir |

Cuando la barra lateral de carpetas es visible, la búsqueda se limita a la carpeta actual y el cuadro de búsqueda muestra un prefijo de solo lectura `path:/carpeta/actual`.

**Modo SSH**
| Tecla | Acción |
|-------|--------|
| `PageUp` / `PageDown` | Desplazar el historial de la terminal en modo de pantalla normal |
| Arrastrar ratón | Seleccionar texto y copiarlo al soltar en modo de pantalla normal |
| `Ctrl-\` | Cambiar al navegador SFTP |
| `exit` / `Ctrl-D` | Desconectar, volver a la lista de hosts |

**Modo SFTP**
| Tecla | Acción |
|-------|--------|
| `Tab` | Cambiar foco entre paneles local / remoto |
| `Space` | Alternar selección en la fila enfocada |
| `Space` × 2 | Seleccionar el rango inclusivo desde el ancla hasta la fila enfocada |
| `Esc` | Cancelar multi-selección para el panel activo |
| `Enter` | Abrir directorio |
| `Backspace` / `←` | Ir al directorio padre del panel activo |
| `/` | Filtrar la lista del directorio actual para el panel activo |
| `g` | Saltar a una ruta de directorio local o remota |
| `d` | Descargar el elemento remoto seleccionado, o todos los seleccionados en modo multi-selección |
| `u` | Subir el elemento local seleccionado, o todos los seleccionados en modo multi-selección |
| `D` | Eliminar todos los elementos seleccionados en el panel activo |
| `e` | Editar archivo remoto seleccionado localmente |
| `Ctrl-\` | Volver a la shell de SSH |
| `Ctrl-C` × 2 | Volver a la lista de hosts |

Cuando presionas `e` en un archivo remoto, `sush` lo descarga en un espacio de trabajo temporal, lo abre con la aplicación predeterminada del sistema operativo, vigila los cambios y lo sube automáticamente después de cada guardado. La subida automática escribe primero en un archivo remoto temporal, mueve el objetivo antiguo a un lado si es necesario y luego coloca el nuevo archivo en su lugar.

Cuando transfieres un directorio, `sush` preserva el directorio seleccionado en el destino, prepara los directorios anidados primero y luego transfiere los archivos uno por uno mientras el indicador de cola muestra `actual/total` más el porcentaje del archivo activo.

En el modo de multi-selección de SFTP, cada panel mantiene su propio conjunto de selecciones. Presiona `Space` una vez para alternar la fila enfocada y establecer el ancla. Presiona `Space` dos veces rápidamente en otra fila para seleccionar el rango inclusivo entre el ancla y la fila actual. Mientras la multi-selección está activa, la barra de estado cambia a acciones solo por lotes: el panel local muestra `u / D / Esc`, el panel remoto muestra `d / D / Esc`.

Las transferencias ahora se ejecutan a través de una única cola FIFO limitada a la conexión SSH actual. La esquina inferior derecha de Principal, SSH y SFTP muestra un indicador compacto como `↑ 2/10 37%` o `↓ 2/10 37%`, para que las transferencias largas continúen en segundo plano sin ocupar toda la línea de estado. Desconectar la conexión actual vacía la cola.

Para archivos normales, las descargas repetidas se reanudan desde el tamaño del objetivo local existente cuando este es menor o igual a la fuente remota. Las subidas repetidas se reanudan solo cuando el objetivo remoto es más pequeño que la fuente local; si el objetivo remoto ya tiene el tamaño completo o es más grande, `sush` reinicia ese archivo desde cero. Esta primera versión no añade verificación de hash ni registros de reanudación entre reinicios.

**Ajustes y compatibilidad con PuTTY**

Presiona `,` desde la vista principal para abrir Ajustes. El `PuTTY compatibility launcher` está desactivado por defecto. En Windows, presiona `Space` en Ajustes para instalar un shim gestionado en `~/.config/sush/putty-compat/putty.exe`, y luego configura tu cliente de bastión para usar esa ruta exacta de PuTTY. Presiona `Space` nuevamente para desactivar el lanzador y eliminar los archivos shim creados por sush.

El shim soporta los argumentos de lanzamiento de SSH de PuTTY `-ssh`, `-l user`, `-P port`, `-i keyfile`, `-pw password` y `[user@]host`. Se rechazan los modos de PuTTY no soportados, como sesiones guardadas, telnet, raw, rlogin, serial y opciones de reenvío de puertos. macOS y Linux muestran una guía de plataforma en Ajustes y no instalan un shim de PuTTY para Windows.

---

## Autenticación

`sush` intenta los métodos de autenticación en este orden:

1. **ssh-agent** — si `SSH_AUTH_SOCK` está configurado, lo utiliza.
2. **IdentityFile** — lee las rutas de las llaves desde tu `~/.ssh/config`; solicita la frase de contraseña si es necesario.
3. **Password** — muestra un prompt de contraseña en la TUI si todo lo demás falla.

---

## Cómo funciona

`sush` utiliza un **emulador de terminal embebido** (impulsado por [alacritty_terminal](https://github.com/alacritty/alacritty)). Cuando te conectas a un host, `sush` alimenta la salida de la PTY remota en una máquina de estado VT100/xterm en el proceso y renderiza el resultado como un widget de ratatui, de modo que la interfaz de sush (barra de estado, pistas de teclas) permanece visible durante toda la sesión.

- Los programas de terminal (`vim`, `tmux`, `htop`) funcionan correctamente mediante la emulación completa de VT100.
- `Ctrl-\` es interceptado como una tecla de prefijo dentro de la TUI; todo lo demás se reenvía al remoto.
- La salida de pantalla normal mantiene el historial de scrollback; PageUp/PageDown y la rueda del ratón mueven el desplazamiento visible.
- La selección arrastrando el ratón se resalta localmente y se copia al portapapeles del sistema al soltar.
- SSH y SFTP comparten la misma conexión TCP a través de canales separados; el cambio es instantáneo y no requiere re-autenticación.

---

## Hoja de ruta

| Versión | Enfoque |
|---------|-------|
| **v0.1** ✅ | Conexión SSH · Navegador SFTP · subida/descarga · cambio con `Ctrl-\` |
| **v0.2** ✅ | Emulador de terminal embebido · TUI visible durante sesiones SSH |
| **v0.3** ✅ | Editor de hosts TUI · editor de chips de etiquetas · importación manual de config SSH |
| **v0.4** ✅ | Historial de conexiones · búsqueda potenciada por recencia · prueba de conectividad TCP |
| **v0.5** ✅ | Etiquetas de tipo ruta · barra lateral de carpetas en vista principal · salto de carpeta · búsqueda `path:` limitada |
| **v0.6** ✅ | Almacenamiento de credenciales en el llavero del sistema · guardado silencioso tras auth exitosa · entrada temporal solo cuando Secret Service no está disponible |
| **v0.7** ✅ | Transferencia recursiva de carpetas con progreso agregado · edición de archivos remotos con subida automática al guardar · SFTP de doble panel · cola de transferencia en segundo plano · soporte de reanudación |
| **v0.8** ✅ | Gestor de reenvío de puertos · ProxyJump de un salto · proxy SOCKS5 · vista de estado del túnel |
| **v1.0** ✅ | Smoke test en macOS · lanzamientos de binarios vía GitHub Actions · consistencia de documentación |
| **v1.1** ✅ | Scrollback de SSH · copia de selección con ratón · navegación al padre en SFTP · búsqueda de lista · ir a ruta |
| **v1.2** ✅ | Verificación de lanzamientos · guardia de versión · script de instalación · borrador de fórmula Homebrew · docs de checksum |

---

## Construido con

- [russh](https://github.com/Eugeny/russh) — implementación de SSH pura en Rust.
- [ratatui](https://ratatui.rs) — framework de interfaz de usuario para terminal.
- [nucleo](https://github.com/helix-editor/nucleo) — buscador difuso (el mismo que usa Helix).
- [tokio](https://tokio.rs) — runtime asíncrono.

Binario único. Sin dependencias del sistema. Sin libssh2. Sin OpenSSL.

---

## Licencia

MIT
