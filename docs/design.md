# RUST SSH MANAGER

## System Design & Technical Specification — v1.0

**Platform:** Ubuntu Linux
**Programming Language:** Rust
**Interface:** Terminal User Interface (TUI)
**Design Reference:** lazydocker, lazygit, Midnight Commander
**Architecture:** Modular Monolith, Event-Driven
**Input:** Hybrid Mouse + Keyboard
**SSH Backend:** System OpenSSH
**File Transfer:** SFTP

---

# 1. Tổng quan dự án

## 1.1. Mục tiêu

Xây dựng một ứng dụng quản lý SSH và tệp từ xa, lấy cảm hứng từ MobaXterm nhưng đơn giản hơn, chạy trực tiếp trong terminal Ubuntu.

Ứng dụng tập trung vào bốn chức năng:

| Module            | Chức năng                                |
| ----------------- | ---------------------------------------- |
| Host Manager      | Quản lý SSH hosts và cấu hình OpenSSH    |
| Terminal Emulator | Thao tác local shell và remote SSH shell |
| File Manager      | Duyệt, lựa chọn và quản lý file/folder   |
| File Transfer     | Upload/download file và folder qua SFTP  |

Ứng dụng phải hỗ trợ chuột và bàn phím đồng thời, không cần chuyển đổi giữa hai chế độ đầu vào.

## 1.2. Nguyên tắc thiết kế

**Simple:** Giao diện tối giản, tránh các thành phần không cần thiết.

**Native:** Chạy trực tiếp trên Ubuntu, không phụ thuộc Electron, Chromium, GTK hoặc Qt.

**Compatible:** Sử dụng SSH config chuẩn để các host có thể truy cập từ terminal bên ngoài ứng dụng.

**Non-blocking:** Kết nối mạng và truyền tệp không được làm treo giao diện.

**Keyboard-first, Mouse-friendly:** Mọi thao tác chính đều có phím tắt, đồng thời hỗ trợ click chuột.

**Secure by default:** Tận dụng OpenSSH, SSH agent, known_hosts và các cơ chế bảo mật sẵn có.

## 1.3. Phạm vi

**Trong phạm vi phiên bản v1.0:**

- Quản lý SSH hosts.
- Tạo và chỉnh sửa cấu hình SSH.
- SSH vào nhiều server.
- Local và remote terminal.
- Giao diện TUI hỗ trợ chuột.
- Dual-pane File Manager.
- Duyệt thư mục local và remote.
- Upload/download file và folder.
- Rename, delete, refresh, tạo thư mục.
- Chọn nhiều file/folder.
- Hiển thị tiến độ truyền tệp.

**Chưa nằm trong phạm vi:**

- RDP/VNC.
- X11 Forwarding UI.
- SSH tunneling manager.
- Đồng bộ thư mục tự động.
- Trình chỉnh sửa code tích hợp.
- Hệ thống plugin.
- Quản lý mật khẩu riêng.
- Trình quản lý tệp GUI độc lập.

---

# 2. Kiến trúc tổng thể

Ứng dụng sử dụng một Rust binary duy nhất, chia thành nhiều module nội bộ.

```text
                          UBUNTU
┌────────────────────────────────────────────────────────┐
│                                                        │
│                    RUST SSH MANAGER                    │
│                                                        │
│  ┌──────────────────────────────────────────────────┐  │
│  │                   UI LAYER                       │  │
│  │                                                  │  │
│  │              Ratatui + Crossterm                 │  │
│  │                                                  │  │
│  │    Hosts    Terminal    Files    Transfers       │  │
│  └────────────────────────┬─────────────────────────┘  │
│                           │                            │
│  ┌────────────────────────▼─────────────────────────┐  │
│  │                APPLICATION CORE                  │  │
│  │                                                  │  │
│  │  App State / Action Dispatcher / Event Handler   │  │
│  │                                                  │  │
│  │              Mouse + Keyboard Input              │  │
│  └────────────────────────┬─────────────────────────┘  │
│                           │                            │
│  ┌────────────────────────▼─────────────────────────┐  │
│  │                  SERVICE LAYER                   │  │
│  │                                                  │  │
│  │  SSH Config │ Session │ PTY │ SFTP │ Transfer    │  │
│  └──────┬──────────┬────────┬──────────┬─────────────┘  │
│         │          │        │          │                │
│         ▼          ▼        ▼          ▼                │
│    ~/.ssh/      OpenSSH    Local     SFTP Client        │
│     config      Process    Shell                        │
│                    │                  │                │
└────────────────────┼──────────────────┼────────────────┘
                     │                  │
                     └────────┬─────────┘
                              │
                              ▼
                       REMOTE SERVER
                     ┌─────────────────┐
                     │                 │
                     │ SSH Server      │
                     │                 │
                     │ Remote Shell    │
                     │                 │
                     │ SFTP Subsystem  │
                     │                 │
                     │ Remote Files    │
                     └─────────────────┘
```

## 2.1. Các lớp kiến trúc

| Layer              | Trách nhiệm                                    |
| ------------------ | ---------------------------------------------- |
| UI Layer           | Hiển thị giao diện và nhận đầu vào             |
| Application Core   | Quản lý state, focus và điều phối action       |
| Service Layer      | Thực thi nghiệp vụ SSH, terminal và filesystem |
| System Integration | Giao tiếp OpenSSH, PTY và filesystem Ubuntu    |
| Remote Services    | SSH server và SFTP subsystem trên máy chủ      |

UI không trực tiếp thực thi các thao tác SSH hoặc truyền tệp.

Mọi tác vụ I/O đều được chuyển cho service layer.

---

# 3. Thiết kế giao diện

## 3.1. Main Layout

Giao diện tham khảo lazydocker, sử dụng tab và panel.

```text
┌──────────────────────── RUST SSH MANAGER ─────────────────────────┐
│                                                                  │
│ [1] Hosts     [2] Terminal     [3] Files       [4] Transfers      │
│                                                                  │
├───────────────────────┬──────────────────────────────────────────┤
│                       │                                          │
│ SSH HOSTS             │ TERMINAL                                 │
│                       │                                          │
│ ● production          │ ubuntu@production:~$                     │
│ ● staging             │                                          │
│ ○ development         │ $ ls -la                                 │
│                       │                                          │
│                       │ drwxr-xr-x  projects                     │
│                       │ -rw-r--r--  README.md                    │
│                       │                                          │
│                       │ ubuntu@production:~$ █                   │
│                       │                                          │
├───────────────────────┴──────────────────────────────────────────┤
│ [a] Add  [e] Edit  [Enter] Connect  [f] Files  [q] Quit          │
└──────────────────────────────────────────────────────────────────┘
```

Đặc điểm:

- Thanh điều hướng ở phía trên.
- Nội dung thay đổi theo tab.
- Thanh phím tắt ở phía dưới.
- Có trạng thái kết nối.
- Hỗ trợ chuột và bàn phím.
- Tự điều chỉnh kích thước khi terminal được resize.

## 3.2. Các màn hình

| Screen    | Nội dung                         |
| --------- | -------------------------------- |
| Hosts     | Danh sách và quản lý SSH hosts   |
| Terminal  | Local/remote shell sessions      |
| Files     | Dual-pane File Manager           |
| Transfers | Danh sách tác vụ upload/download |

Người dùng có thể click tab hoặc sử dụng phím tắt điều hướng.

Khi terminal đang nhận input trực tiếp, các phím số phải được gửi cho shell, không được tự động chuyển tab.

---

# 4. Input System — Hybrid Mouse + Keyboard

## 4.1. Yêu cầu

Chuột và bàn phím phải điều khiển cùng một trạng thái ứng dụng.

Ví dụ:

```text
Mouse Click: Select README.md
                    │
                    ▼
             App Selection
                    │
                    ▼
Keyboard Arrow Down
                    │
                    ▼
             Select main.rs
```

Không có trạng thái chọn độc lập giữa chuột và bàn phím.

Người dùng có thể luân phiên các thao tác mà không phải bật/tắt một chế độ điều khiển riêng.

## 4.2. Input Architecture

```text
                  USER INPUT
                      │
             ┌────────┴────────┐
             │                 │
             ▼                 ▼
          Keyboard           Mouse
             │                 │
             └────────┬────────┘
                      │
                      ▼
                   Crossterm
                      │
                      ▼
                 Event Handler
                      │
                      ▼
                 Input Router
                      │
                      ▼
                Action Dispatcher
                      │
                      ▼
                   App State
                      │
                      ▼
                    Ratatui
```

## 4.3. Focus Management

Các thành phần có thể nhận focus:

```rust
enum FocusTarget {
    HostList,
    Terminal,
    LocalFiles,
    RemoteFiles,
    PathInput,
    ContextMenu,
    Dialog,
}
```

Quy tắc:

- Click vào Local Panel → focus chuyển sang Local.
- Click vào Remote Panel → focus chuyển sang Remote.
- Nhấn Tab khi ở File Manager → chuyển focus giữa hai panel.
- Click vào đường dẫn → mở chế độ điều hướng/chỉnh sửa đường dẫn.
- Mở popup → popup nhận focus.
- Đóng popup → focus trở về panel trước đó.

### Terminal Focus

Terminal cần cách xử lý riêng vì các phím được gửi trực tiếp đến chương trình đang chạy trong PTY.

Khi Terminal nhận focus, các phím Enter, F2, Delete, F5, Ctrl+C... được gửi đến shell hoặc ứng dụng bên trong.

Sử dụng `Ctrl+]` làm phím thoát khỏi Terminal Focus để quay về điều khiển TUI.

Đây là chế độ định tuyến input cho PTY, không phải chế độ chuyển đổi giữa chuột và bàn phím.

---

# 5. SSH Host Manager

## 5.1. Giao diện

```text
┌──────────────────────── HOST MANAGER ─────────────────────────┐
│                                                              │
│ SSH HOSTS                                                    │
│                                                              │
│ > production                 ubuntu@192.168.1.100            │
│   staging                    ubuntu@192.168.1.101            │
│   development                dev@192.168.1.102               │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│ [Add] [Edit] [Delete] [Connect] [Open Files]                 │
├──────────────────────────────────────────────────────────────┤
│ a Add | e Edit | Del Delete | Enter Connect | / Search       │
└──────────────────────────────────────────────────────────────┘
```

## 5.2. Add Host

```text
┌──────────────────────── ADD SSH HOST ────────────────────────┐
│                                                              │
│ Host Alias:     production                                   │
│ HostName:       192.168.1.100                                │
│ User:           ubuntu                                       │
│ Port:           22                                           │
│ IdentityFile:   ~/.ssh/id_ed25519                            │
│                                                              │
│                  [Save]   [Cancel]                            │
└──────────────────────────────────────────────────────────────┘
```

Các trường:

| Field        | Bắt buộc | Mô tả                      |
| ------------ | -------- | -------------------------- |
| Host Alias   | Có       | Tên sử dụng với lệnh `ssh` |
| HostName     | Có       | IP hoặc domain             |
| User         | Có       | SSH username               |
| Port         | Không    | Mặc định 22                |
| IdentityFile | Không    | SSH private key path       |

## 5.3. SSH Config Integration

Ứng dụng sử dụng cấu hình OpenSSH chuẩn.

Trong:

```text
~/.ssh/config
```

Thêm ở phần đầu của file:

```sshconfig
Include ~/.ssh/rust-ssh-manager/hosts.conf
```

Các host do ứng dụng quản lý được ghi vào:

```text
~/.ssh/rust-ssh-manager/hosts.conf
```

Ví dụ:

```sshconfig
Host production
    HostName 192.168.1.100
    User ubuntu
    Port 22
    IdentityFile ~/.ssh/id_ed25519

Host staging
    HostName 192.168.1.101
    User ubuntu
    Port 22
    IdentityFile ~/.ssh/id_ed25519
```

Sau khi lưu, có thể sử dụng bên ngoài ứng dụng:

```bash
ssh production
ssh staging
sftp production
```

## 5.4. Quy tắc quản lý

Ứng dụng không được ghi đè toàn bộ SSH config hiện có.

Các host nằm ngoài file quản lý của ứng dụng có thể được hiển thị nhưng chỉ được sửa sau khi người dùng chủ động import.

Khi thêm hoặc sửa host:

1. Kiểm tra alias hợp lệ.
2. Phát hiện alias trùng hoặc xung đột cấu hình.
3. Sao lưu file cấu hình nếu cần.
4. Ghi cấu hình bằng cơ chế atomic write.
5. Kiểm tra cú pháp và cấu hình hiệu lực bằng `ssh -G <alias>`.
6. Thông báo kết quả cho người dùng.

File cấu hình phải được giới hạn quyền truy cập thích hợp.

Workspace Hosts có `Test connection` chạy nền với BatchMode và timeout 10 giây, giữ nguyên xác minh `known_hosts` và không yêu cầu nhập mật khẩu trong TUI. Docker container được lưu trong `config.toml` dưới dạng `[[docker_hosts]]`, gắn với SSH alias, container name và shell `auto`, `bash` hoặc `sh`.

Danh sách Hosts hiển thị Docker host thành các dòng con của SSH host tương ứng. Có thể thu gọn/mở nhóm bằng `←`/`→` hoặc click biểu tượng disclosure; cuộn chuột đi từng dòng đang hiển thị. Chỉnh sửa SSH host được quản lý sẽ cập nhật SSH alias tham chiếu của Docker con. Xóa SSH host yêu cầu xác nhận và đồng thời xóa cấu hình Docker con khỏi ứng dụng; không xóa container trên máy chủ từ xa và không làm gián đoạn terminal session đang chạy. Các nguồn kết quả của Hosts (test, import, list, add/edit/delete) hiển thị trong `Host Details`.

Host do ứng dụng quản lý có metadata `created_at` và `updated_at` (Unix timestamp) trong `config.toml`; UI hiển thị theo múi giờ Việt Nam. Host cũ chưa có metadata hiển thị `Không rõ`. OpenSSH host chỉ đọc không nhận metadata cho tới khi được import.

---

# 6. Terminal Emulator

## 6.1. Yêu cầu

Hỗ trợ:

- Local Bash/Zsh shell.
- Remote SSH shell.
- Nhiều terminal session.
- Terminal tabs.
- ANSI escape sequences.
- Ứng dụng tương tác như Vim, Nano, htop.
- Terminal resize.
- Scrollback.
- Keyboard input forwarding.
- Terminal mouse reporting khi chương trình bên trong hỗ trợ.

## 6.2. Terminal Architecture

```text
                   TUI
                    │
                    ▼
               Ratatui Widget
                    │
                    ▼
                  tui-term
                    │
                    ▼
                VT100 Parser
                    ▲
                    │
                PTY Master
                    │
                PTY Slave
                    │
            ┌───────┴────────┐
            │                │
            ▼                ▼
        Local Shell      SSH Process
                             │
                             ▼
                       Remote Shell
```

## 6.3. Terminal Session

Mỗi phiên terminal có:

```rust
struct TerminalSession {
    id: SessionId,
    host_alias: Option<String>,
    title: String,
    status: SessionStatus,
}
```

Một session cần quản lý thêm các tài nguyên runtime gồm:

- PTY master.
- Child process.
- Input writer.
- Output reader.
- VT100 parser và screen buffer.
- Terminal size.
- Scrollback state.

Các tài nguyên này nên được giữ trong `TerminalManager`, thay vì đặt trực tiếp toàn bộ vào UI state.

## 6.4. Luồng SSH Terminal

```text
User selects Host
        │
        ▼
Create PTY
        │
        ▼
Spawn: ssh production
        │
        ▼
OpenSSH Authentication
        │
        ▼
Remote Shell
        │
        ▼
PTY Output
        │
        ▼
VT100 Parser
        │
        ▼
Ratatui Render
```

Người dùng có thể chạy:

```bash
ls -la
vim main.rs
htop
docker ps
```

Ứng dụng không trực tiếp thực thi các lệnh này.

Ứng dụng đóng vai trò terminal emulator và chuyển dữ liệu giữa PTY với giao diện.

## 6.5. Điều khiển

Khi đang sử dụng Terminal:

- Phím bình thường chuyển đến PTY; `Ctrl+T` được giao diện giữ lại để kiểm tra host đang chọn trong Hosts.
- Terminal resize cập nhật kích thước PTY.
- Click vào terminal để nhận focus.
- Mouse events được chuyển đến ứng dụng bên trong nếu ứng dụng đó bật mouse reporting.
- `Ctrl+]` trả quyền điều khiển về giao diện chính.

Phím tắt workspace dùng `Alt+H` Hosts, `Alt+Shift+H` History, `Alt+T` Terminal, `Alt+Shift+T` Transfers, `Alt+F` Files và `Alt+C` Configuration; `Alt+←/→` đi tuần tự. `Tab` không dùng để đổi workspace.

Terminal cần xử lý đúng các chuỗi điều khiển, phím chức năng và các chế độ nhập của chương trình bên trong.

---

# 7. File Manager — Dual Pane

Đây là màn hình trung tâm cho chức năng duyệt tệp và truyền file.

## 7.1. Giao diện

```text
┌──────────────────────────── FILE MANAGER ───────────────────────────┐
│ Connected: production                           ● Connected         │
├────────────────────────────────┬───────────────────────────────────┤
│ LOCAL                          │ REMOTE                            │
│                                │                                   │
│ /home/user/projects/           │ /var/www/html/                    │
├────────────────────────────────┼───────────────────────────────────┤
│ Name                Size       │ Name                 Size          │
│                                │                                   │
│ 📁 src/              <DIR>     │ 📁 public/            <DIR>        │
│ 📁 target/           <DIR>     │ 📁 uploads/           <DIR>        │
│ > 📄 Cargo.toml      1 KB      │ 📄 index.html         4 KB         │
│ 📄 README.md         2 KB      │ 📄 nginx.conf         2 KB         │
│                                │                                   │
│                                │                                   │
│                                │                                   │
├────────────────────────────────┴───────────────────────────────────┤
│ Selected: Cargo.toml                                              │
│                                                                    │
│ [Upload →] [← Download] [Rename] [Delete] [Refresh] [New Folder]  │
├────────────────────────────────────────────────────────────────────┤
│ F2 Rename | Del Delete | F5 Refresh | F7 Mkdir | Tab Switch       │
└────────────────────────────────────────────────────────────────────┘
```

## 7.2. Local Panel

Sử dụng filesystem API của Rust/Tokio để duyệt file trên Ubuntu.

Hỗ trợ:

- Đọc thư mục.
- Hiển thị file/folder.
- Di chuyển giữa các thư mục.
- Đổi tên.
- Xóa.
- Tạo thư mục.
- Chọn nhiều file.
- Hiển thị metadata.

## 7.3. Remote Panel

Sử dụng SFTP để thực hiện các thao tác tương tự trên máy chủ.

Hai panel dùng chung một giao diện dữ liệu, nhưng backend thực thi khác nhau.

Files chỉ tải dữ liệu khi có terminal session active. SSH session dùng chung OpenSSH control socket của terminal; Docker session chạy file operations và truyền file qua `docker exec` trên SSH host. Nếu container thiếu tiện ích cần thiết, giao diện báo danh sách và gợi ý lệnh cài theo package manager, không tự cài. Local terminal chỉ có pane local. Mỗi session giữ đường dẫn local và remote riêng; lệnh `cd` trong terminal không đổi đường dẫn Files.

```text
             FILE MANAGER
                   │
                   ▼
              FILE OPERATIONS
                   │
          ┌────────┴────────┐
          │                 │
          ▼                 ▼
     LOCAL BACKEND     REMOTE BACKEND
          │                 │
          ▼                 ▼
     Ubuntu FS API       SFTP API
```

## 7.4. Điều hướng đường dẫn

Cả hai panel đều có thanh địa chỉ.

Hỗ trợ hai cách:

**Breadcrumb Navigation**

```text
Home > user > projects > myapp > src
```

Click vào một thành phần để chuyển đến thư mục tương ứng.

**Path Input**

Sử dụng `Ctrl+L` hoặc click thanh địa chỉ để nhập trực tiếp:

```text
/home/user/projects/myapp/src
```

Nhấn Enter để mở đường dẫn, Esc để hủy.

## 7.5. Mouse Operations

| Mouse Action     | Hành vi                      |
| ---------------- | ---------------------------- |
| Left Click       | Chọn file/folder             |
| Double Click     | Mở folder hoặc xem file      |
| Right Click      | Mở context menu              |
| Scroll           | Cuộn danh sách               |
| Ctrl + Click     | Chọn/bỏ chọn một mục         |
| Shift + Click    | Chọn một dải mục             |
| Click Breadcrumb | Điều hướng thư mục           |
| Click Path       | Nhập đường dẫn               |
| Click Panel      | Chuyển focus                 |
| Click Button     | Thực thi hành động tương ứng |

Double-click được phát hiện bằng thời gian giữa hai lần click liên tiếp trên cùng một mục.

Để hỗ trợ chọn nhiều mục, application state phải phân biệt:

- Focused item.
- Selection anchor.
- Selected items.

## 7.6. Keyboard Operations

Đây là bảng phím tắt chính thức của File Manager.

| Phím             | Hành động                   |
| ---------------- | --------------------------- |
| **F2**           | **Rename file/folder**      |
| **Delete / Del** | **Delete file/folder**      |
| **F5**           | **Refresh panel**           |
| F6               | Transfer sang panel còn lại |
| F7               | Create folder               |
| Enter            | Open folder / view file     |
| Ctrl+Enter       | Edit file with configured editor |
| Backspace        | Parent directory            |
| ↑ / ↓            | Move selection cursor       |
| PageUp/PageDown  | Scroll                      |
| Space            | Toggle multi-selection      |
| Tab              | Switch Local/Remote         |
| Ctrl+L           | Edit current path           |
| /                | Search trong danh sách      |
| Esc              | Cancel/Close popup          |

**F5 chỉ dùng để Refresh khi File Manager có focus. Không sử dụng F5 để copy hoặc transfer.**

## 7.7. Rename — F2

Khi chọn file và nhấn F2:

```text
┌────────────────────── RENAME ───────────────────────┐
│                                                     │
│ Current Name: Cargo.toml                            │
│                                                     │
│ New Name:     Cargo.toml█                           │
│                                                     │
│                 [Rename] [Cancel]                   │
└─────────────────────────────────────────────────────┘
```

Hành vi:

- Tự động điền tên hiện tại.
- Nhấn Enter để xác nhận.
- Nhấn Esc để hủy.
- Thông báo nếu tên không hợp lệ hoặc bị trùng.
- Không tự ý ghi đè đối tượng đã tồn tại.
- Giữ lựa chọn trên mục vừa đổi tên khi hoàn tất.

## 7.8. Delete — Del

Khi nhấn Delete:

```text
┌──────────────────── DELETE CONFIRMATION ────────────────────┐
│                                                             │
│ Delete 3 selected items?                                    │
│                                                             │
│  - Cargo.toml                                               │
│  - README.md                                                │
│  - src/                                                     │
│                                                             │
│ This operation may be irreversible.                         │
│                                                             │
│                [Delete] [Cancel]                            │
└─────────────────────────────────────────────────────────────┘
```

Quy tắc:

- Luôn xác nhận trước khi xóa.
- Hỗ trợ nhiều file/folder.
- Phân biệt file và thư mục.
- Không tự động đi theo symbolic link khi xóa đệ quy.
- Hiển thị lỗi nếu không có quyền truy cập.
- Không mặc định xóa thư mục đang được sử dụng làm thư mục gốc của panel.

Ở MVP, Delete là thao tác xóa thực sự, không giả định remote server có Trash.

## 7.9. Refresh — F5

Khi nhấn F5:

```text
F5
 │
 ▼
Current Focused Panel
 │
 ▼
Reload Directory
 │
 ▼
Update File List
 │
 ▼
Restore Selection
 │
 ▼
Render UI
```

Yêu cầu:

- Refresh panel hiện tại.
- Không chuyển thư mục.
- Không reset focus.
- Giữ vị trí scroll nếu còn hợp lệ.
- Giữ các file đang chọn nếu chúng vẫn tồn tại.
- Thực hiện bất đồng bộ đối với Remote Panel.
- Hiển thị trạng thái Loading khi cần.

## 7.10. Right-click Context Menu

Menu thay đổi theo panel.

Local:

```text
┌───────────────────┐
│ Open              │
│ Upload →          │
│ Copy Path         │
│ Rename            │
│ Delete            │
│ Properties        │
└───────────────────┘
```

Remote:

```text
┌───────────────────┐
│ Open              │
│ ← Download        │
│ Copy Path         │
│ Rename            │
│ Delete            │
│ Properties        │
└───────────────────┘
```

Click chuột phải lên mục chưa được chọn sẽ focus và chọn mục đó trước khi mở menu.

Nếu click chuột phải lên một mục thuộc tập multi-selection, menu sẽ thao tác trên tập đã chọn.

---

# 8. File Transfer

## 8.1. Mục tiêu

Hỗ trợ truyền file và folder hai chiều.

```text
       LOCAL                        REMOTE
         │                            │
         ├──────── UPLOAD ───────────►│
         │                            │
         │◄──────── DOWNLOAD ─────────┤
         │                            │
```

Các thao tác phải có thể được kích hoạt bằng:

- Click vào nút Upload/Download.
- Context menu.
- Phím F6 khi File Manager có focus.
- Kéo thả giữa hai panel khi terminal hỗ trợ.

## 8.2. Internal Drag & Drop

Đối với TUI, thiết kế kéo thả được thực hiện trong phạm vi ứng dụng.

Ví dụ:

```text
LOCAL                            REMOTE
┌─────────────────┐             ┌──────────────────┐
│                 │             │                  │
│ 📄 config.json  │────────────►│ /var/www/app/    │
│                 │             │                  │
└─────────────────┘             └──────────────────┘
```

Luồng xử lý:

```text
Mouse Down on File
        │
        ▼
Start Drag Candidate
        │
        ▼
Mouse Drag
        │
        ▼
Show Transfer Target
        │
        ▼
Mouse Up on Opposite Panel
        │
        ▼
Validate Destination
        │
        ▼
Transfer Confirmation
        │
        ▼
Start Transfer
```

Bên trong cùng panel, thao tác kéo thả sẽ không mặc định di chuyển file.

Trong phiên bản đầu, kéo thả chỉ được sử dụng để copy từ Local sang Remote hoặc từ Remote về Local.

Cần phân biệt:

**Internal Drag & Drop:** Kéo file giữa hai panel trong ứng dụng TUI.

**Desktop Drag & Drop:** Kéo file từ Nautilus hoặc ứng dụng bên ngoài vào terminal.

Loại thứ hai không được đảm bảo hỗ trợ, vì nó phụ thuộc vào terminal emulator và cơ chế tích hợp với desktop.

Do đó, Internal Drag & Drop là tính năng nâng cao; người dùng luôn có thể sử dụng nút Upload/Download hoặc F6 thay thế.

## 8.3. Transfer Engine

```text
                 TRANSFER REQUEST
                        │
                        ▼
                 Transfer Manager
                        │
                        ▼
                  Transfer Queue
                        │
                        ▼
                 Background Worker
                        │
             ┌──────────┴─────────┐
             │                    │
             ▼                    ▼
          Upload               Download
             │                    │
             └──────────┬─────────┘
                        │
                        ▼
                   SFTP Client
                        │
                        ▼
                    Progress
                        │
                        ▼
                      UI
```

## 8.4. Transfer States

```rust
enum TransferStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}
```

Mỗi transfer có:

```rust
struct TransferTask {
    id: TransferId,
    direction: TransferDirection,
    source: PathBuf,
    destination: PathBuf,
    total_bytes: Option<u64>,
    transferred_bytes: u64,
    status: TransferStatus,
}
```

Đường dẫn local và remote phải được phân biệt rõ trong mô hình nghiệp vụ, kể cả khi cùng sử dụng kiểu dữ liệu `PathBuf`.

## 8.5. Transfer Progress

```text
┌──────────────────────── TRANSFERS ───────────────────────────┐
│                                                              │
│ Uploading application.tar.gz                                 │
│                                                              │
│ [████████████████████░░░░░░░░░░] 68%                         │
│                                                              │
│ 680 MB / 1 GB                                                │
│                                                              │
│ [Cancel]                                                     │
└──────────────────────────────────────────────────────────────┘
```

Trạng thái cần thể hiện:

- File đang truyền.
- Tổng dung lượng nếu xác định được.
- Số byte đã truyền.
- Phần trăm tiến độ.
- Trạng thái hoàn thành/thất bại.
- Tùy chọn hủy tác vụ.

## 8.6. Transfer Safety

Các nguyên tắc bắt buộc:

**Streaming:** Truyền theo chunk, không đọc toàn bộ file vào RAM.

**Temporary File:** Truyền vào file tạm trước khi thay thế file đích.

**Overwrite Confirmation:** Không ghi đè file nếu chưa được người dùng đồng ý.

**Cancellation:** Hỗ trợ hủy transfer và xử lý file tạm.

**Recursive Folder Transfer:** Hỗ trợ sao chép thư mục và các thư mục con.

**Symlink Safety:** Không mặc định đi theo symbolic link khi duyệt đệ quy.

**Error Handling:** Thông báo rõ khi thiếu quyền, mất kết nối hoặc hết dung lượng.

Cơ chế rename/replace sau transfer phải dựa trên khả năng của filesystem và SFTP server, không giả định mọi server đều hỗ trợ atomic replacement.

---

# 9. SFTP Client

## 9.1. Thiết kế

Sử dụng:

```text
openssh
    +
openssh-sftp-client
```

Lớp SFTP cung cấp các thao tác chung cho Remote File Manager.

```rust
trait FileSystemBackend {
    // Conceptual interface:
    // list_directory()
    // create_directory()
    // rename()
    // remove_file()
    // remove_directory()
    // metadata()
}
```

Khi triển khai, interface bất đồng bộ sẽ được thiết kế phù hợp với Tokio và các kiểu dữ liệu thực tế.

## 9.2. Các thao tác hỗ trợ

| Operation        | Local Backend | Remote Backend |
| ---------------- | ------------- | -------------- |
| List directory   | Rust/Tokio FS | SFTP           |
| Read metadata    | Rust/Tokio FS | SFTP           |
| Create directory | Rust/Tokio FS | SFTP           |
| Rename           | Rust/Tokio FS | SFTP           |
| Delete           | Rust/Tokio FS | SFTP           |
| Read file        | Rust/Tokio FS | SFTP           |
| Write file       | Rust/Tokio FS | SFTP           |

## 9.3. SSH Authentication

MVP ưu tiên:

- OpenSSH public-key authentication.
- SSH agent.
- Existing SSH config.
- Existing known_hosts.

Với terminal, SSH có thể yêu cầu nhập password hoặc passphrase thông qua PTY.

Với SFTP, phiên kết nối được quản lý riêng và ưu tiên xác thực bằng SSH key/agent.

Không giả định rằng đăng nhập terminal bằng password sẽ tự động cấp quyền xác thực cho một SFTP connection khác.

MVP không lưu trữ mật khẩu SSH.

---

# 10. Application State & Event System

## 10.1. Event-driven Architecture

```text
                INPUT EVENTS
                     │
                     ▼
               Event Router
                     │
                     ▼
                  Actions
                     │
                     ▼
                 App Update
                     │
             ┌───────┴────────┐
             │                │
             ▼                ▼
         UI State       Async Commands
             │                │
             │                ▼
             │           Service Tasks
             │                │
             │                ▼
             │           App Messages
             │                │
             └────────┬───────┘
                      │
                      ▼
                  UI Render
```

Mọi kết quả từ tác vụ bất đồng bộ được gửi về application loop dưới dạng message.

Điều này giúp bảo đảm các thao tác mạng không chặn giao diện.

## 10.2. App State

Mô hình dữ liệu khái niệm:

```rust
struct App {
    active_screen: Screen,
    focus: FocusTarget,
    hosts: HostState,
    terminals: TerminalState,
    files: FileManagerState,
    transfers: TransferState,
    dialog: Option<DialogState>,
}
```

File Manager lưu trạng thái riêng cho từng panel:

```rust
struct FilePanelState {
    current_path: PathBuf,
    entries: Vec<FileEntry>,
    cursor_index: usize,
    selected_items: HashSet<PathBuf>,
    scroll_offset: usize,
    loading: bool,
}
```

```rust
struct FileManagerState {
    local: FilePanelState,
    remote: FilePanelState,
    active_panel: FilePanel,
    connected_host: Option<String>,
}
```

`selected_items` không phụ thuộc vào mouse hay keyboard.

## 10.3. Actions

Sử dụng một enum chung để biểu diễn hành động từ người dùng.

```rust
enum Action {
    SelectFile,
    OpenSelected,
    ParentDirectory,
    SwitchPanel,
    EditPath,

    RenameSelected,
    DeleteSelected,
    RefreshDirectory,
    CreateDirectory,

    UploadSelected,
    DownloadSelected,

    ConnectHost,
    DisconnectHost,
}
```

Input router chuyển sự kiện chuột/bàn phím thành action tương ứng.

Ví dụ:

```text
F2
 │
 ▼
Action::RenameSelected
 │
 ▼
Rename Dialog
```

Hoặc:

```text
Right Click
     │
     ▼
Context Menu
     │
     ▼
Click Rename
     │
     ▼
Action::RenameSelected
     │
     ▼
Rename Dialog
```

Cả hai cách đều đi qua cùng một luồng nghiệp vụ.

## 10.4. Mouse Hit Testing

Trong mỗi lần render, UI xác định các vùng có thể tương tác:

```text
Host List Rect
Terminal Rect
Local File List Rect
Remote File List Rect
Breadcrumb Rects
Button Rects
Dialog Rect
```

Khi nhận mouse event, ứng dụng sử dụng tọa độ để xác định vùng được click.

Đối với file list, vị trí dòng cần được ánh xạ qua `scroll_offset` để tìm đúng file.

Hit testing phải được cập nhật sau mỗi lần thay đổi layout, resize hoặc chuyển screen.

---

# 11. Technology Stack

## 11.1. Core Dependencies

| Crate                 | Mục đích                        |
| --------------------- | ------------------------------- |
| `ratatui`             | Render TUI                      |
| `crossterm`           | Keyboard/mouse events           |
| `tokio`               | Async runtime                   |
| `portable-pty`        | Local/remote terminal PTY       |
| `tui-term`            | Terminal widget                 |
| `vt100`               | Terminal escape-sequence parser |
| `openssh`             | OpenSSH session integration     |
| `openssh-sftp-client` | SFTP operations                 |
| `anyhow`              | Application error handling      |
| `thiserror`           | Typed errors                    |
| `serde`               | Serialization                   |
| `toml`                | Application configuration       |
| `directories`         | XDG-compatible paths            |
| `tracing`             | Logging                         |
| `tempfile`            | Temporary files                 |

Với Ubuntu, cần cài đặt OpenSSH client trên hệ thống.

`portable-pty` được sử dụng cho cả local shell và tiến trình OpenSSH chạy bên trong terminal.

Các thao tác đọc PTY có tính blocking nên cần được chạy bằng thread riêng hoặc `spawn_blocking`, trong khi các tác vụ SFTP và filesystem phù hợp với Tokio async.

## 11.2. Application Configuration

Cấu trúc dữ liệu:

```text
~/.config/fpsxterm/fpsxterm-tui/
└── config.toml

~/.local/state/fpsxterm/fpsxterm-tui/
└── app.log

~/.ssh/
├── config
├── known_hosts
├── id_ed25519
└── rust-ssh-manager/
    └── hosts.conf
```

Ví dụ `config.toml`:

```toml
[ui]
theme = "dark"
mouse_enabled = true
show_hidden_files = false

[terminal]
shell = "/bin/bash"
editor = ["nano"]
scrollback_lines = 10000

[files]
confirm_delete = true
confirm_overwrite = true

[transfer]
max_concurrent = 2
```

Ví dụ Docker container host (có thể lặp lại cho nhiều container):

```toml
[[docker_hosts]]
ssh_host = "production"
container = "web"
shell = "auto" # auto, bash hoặc sh
```

Đây là cấu hình nội bộ của ứng dụng, không thay thế `~/.ssh/config`. Workspace Configuration cung cấp form cho các nhóm trên (Docker hosts vẫn được quản lý ở Hosts); `Ctrl+S` kiểm tra, ghi atomically rồi đọc lại cấu hình. UI và giới hạn transfer áp dụng ngay; session terminal đang chạy và transfer hiện tại tiếp tục, các cấu hình shell/editor/scrollback áp dụng cho session mới. Theme hiện chỉ hỗ trợ `dark`.

Cuộn chuột trong Files tác động lên pane nằm dưới con trỏ; trong Hosts và Configuration, mỗi nấc di chuyển đúng một dòng hoặc trường. Configuration hỗ trợ highlight khi hover và click để chọn trường; cuộn chỉ đổi trường đang chọn, còn sửa đổi chỉ xảy ra khi người dùng bật chế độ chỉnh sửa.
Nếu chưa có file, ứng dụng tự tạo `config.toml` với các giá trị mặc định ở thư mục cấu hình XDG. File được ghi với quyền chỉ chủ sở hữu đọc/ghi trên Unix; ứng dụng từ chối đọc hoặc thay thế file cấu hình qua symbolic link.

Khi thêm hoặc sửa SSH/Docker host, `Ctrl+T` kiểm tra chính các giá trị đang nhập nhưng là tùy chọn; host vẫn có thể lưu khi chưa test hoặc test thất bại. Mọi thay đổi trường dữ liệu sẽ xóa trạng thái thành công cũ để tránh hiển thị kết quả không còn khớp với cấu hình.

---

# 12. Source Code Structure

```text
rust-ssh-manager/
│
├── Cargo.toml
├── Cargo.lock
├── README.md
│
├── src/
│   ├── main.rs
│   ├── app.rs
│   ├── config.rs
│   ├── error.rs
│   │
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── layout.rs
│   │   ├── theme.rs
│   │   ├── hosts.rs
│   │   ├── terminal.rs
│   │   ├── files.rs
│   │   ├── transfers.rs
│   │   ├── dialogs.rs
│   │   └── context_menu.rs
│   │
│   ├── input/
│   │   ├── mod.rs
│   │   ├── keyboard.rs
│   │   ├── mouse.rs
│   │   ├── focus.rs
│   │   └── keybindings.rs
│   │
│   ├── ssh/
│   │   ├── mod.rs
│   │   ├── config.rs
│   │   ├── host.rs
│   │   └── session.rs
│   │
│   ├── terminal/
│   │   ├── mod.rs
│   │   ├── manager.rs
│   │   ├── pty.rs
│   │   └── emulator.rs
│   │
│   ├── filesystem/
│   │   ├── mod.rs
│   │   ├── entry.rs
│   │   ├── local.rs
│   │   ├── remote.rs
│   │   └── browser.rs
│   │
│   ├── sftp/
│   │   ├── mod.rs
│   │   └── client.rs
│   │
│   ├── transfer/
│   │   ├── mod.rs
│   │   ├── manager.rs
│   │   ├── task.rs
│   │   ├── upload.rs
│   │   └── download.rs
│   │
│   └── event/
│       ├── mod.rs
│       ├── action.rs
│       ├── message.rs
│       └── handler.rs
│
└── tests/
    ├── ssh_config.rs
    ├── file_navigation.rs
    ├── file_operations.rs
    ├── input_routing.rs
    └── transfer.rs
```

## 12.1. Module Boundaries

| Module       | Responsibility                    |
| ------------ | --------------------------------- |
| `ui`         | Render giao diện                  |
| `input`      | Điều phối chuột/bàn phím và focus |
| `ssh`        | Quản lý cấu hình và phiên SSH     |
| `terminal`   | PTY và terminal emulator          |
| `filesystem` | Filesystem abstraction            |
| `sftp`       | SFTP connection                   |
| `transfer`   | Copy/upload/download              |
| `event`      | Action/message routing            |
| `app`        | Global application state          |

Không đặt logic upload/download trực tiếp trong `ui/files.rs`.

Không để event handler gọi đồng bộ các thao tác mạng lâu.

Không để terminal parser phụ thuộc vào layout cụ thể của Ratatui.

---

# 13. Security & Reliability

## 13.1. SSH Security

Sử dụng `known_hosts` theo cơ chế OpenSSH.

Không tự động vô hiệu hóa host key verification.

Không lưu private key hoặc mật khẩu trong application config.

Không truyền các giá trị đầu vào của người dùng qua shell command string khi có thể sử dụng argument API.

Các alias SSH cần được kiểm tra để không bị hiểu thành command-line options.

## 13.2. Filesystem Safety

Trước các thao tác nguy hiểm:

- Xác nhận delete.
- Xác nhận overwrite.
- Kiểm tra quyền truy cập.
- Xử lý symbolic link.
- Bảo vệ file tạm.
- Xử lý tình huống destination đã tồn tại.
- Không tự động theo symlink trong quá trình xóa/copy đệ quy.

## 13.3. Runtime Reliability

Ứng dụng cần xử lý:

- SSH disconnect.
- SFTP timeout.
- Terminal child process exit.
- File transfer cancellation.
- Terminal resize.
- File permission errors.
- Network interruption.
- File/folder không còn tồn tại khi người dùng thao tác.

Khi ứng dụng thoát, cần khôi phục terminal về trạng thái ban đầu, bao gồm raw mode, alternate screen và mouse capture.

---

# 14. Development Roadmap

## Phase 1 — TUI Foundation

**Mục tiêu:** Có giao diện ứng dụng hoạt động.

Triển khai:

- Ratatui layout.
- Mouse/keyboard input.
- Focus management.
- Navigation.
- Popup system.
- Application state.
- Event loop.

**Done when:** Có thể chuyển tab, click các panel và thao tác phím tắt mà không gặp xung đột.

## Phase 2 — SSH Host Manager

**Mục tiêu:** Quản lý SSH hosts.

Triển khai:

- Đọc SSH config.
- Hiển thị host.
- Add/Edit/Delete.
- Managed Include file.
- Config validation.
- Connect action.

**Done when:** Thêm host qua TUI và sử dụng được `ssh <alias>` bên ngoài ứng dụng.

## Phase 3 — Terminal Emulator

**Mục tiêu:** Có terminal tương tác.

Triển khai:

- PTY management.
- Local shell.
- Remote SSH shell.
- Terminal rendering.
- Input forwarding.
- Resize handling.
- Multiple sessions.

**Done when:** Chạy được Vim, Nano, htop và shell tương tác trong TUI.

## Phase 4 — File Manager

**Mục tiêu:** Duyệt filesystem bằng chuột và bàn phím.

Triển khai:

- Dual-pane layout.
- Local filesystem.
- SFTP filesystem.
- Mouse selection.
- Keyboard navigation.
- Breadcrumb.
- Path input.
- Multi-selection.
- Context menu.

**Done when:** Người dùng có thể điều hướng Local/Remote bằng cả chuột và bàn phím.

## Phase 5 — File Operations

**Mục tiêu:** Hoàn thiện thao tác quản lý tệp.

Triển khai:

- F2 Rename.
- Delete Confirmation.
- F5 Refresh.
- F7 New Folder.
- File/folder metadata.
- Multi-item operations.

**Done when:** Tất cả thao tác hoạt động ở cả Local và Remote, có xử lý lỗi và xác nhận phù hợp.

## Phase 6 — File Transfer

**Mục tiêu:** Upload/download file và folder.

Triển khai:

- Streaming transfer.
- Recursive transfer.
- Progress reporting.
- Transfer queue.
- Cancellation.
- Overwrite confirmation.
- Error handling.
- Internal drag-and-drop trên terminal tương thích.

**Done when:** Copy được file/folder hai chiều và giao diện vẫn phản hồi trong lúc truyền.

## Phase 7 — Testing & Packaging

**Mục tiêu:** Ổn định và phân phối ứng dụng.

Triển khai:

- Unit tests.
- Integration tests.
- SSH config tests.
- Terminal tests.
- SFTP transfer tests.
- Mouse interaction tests.
- Error recovery tests.
- Release build.
- Ubuntu installation instructions.

**Done when:** Ứng dụng có thể build, cài đặt và chạy ổn định trên Ubuntu.

---

# 15. Acceptance Criteria

Ứng dụng v1.0 được xem là đạt yêu cầu khi đáp ứng các tiêu chí sau:

| ID    | Yêu cầu           | Tiêu chí                                         |
| ----- | ----------------- | ------------------------------------------------ |
| AC-01 | Ubuntu            | Chạy trong terminal Ubuntu                       |
| AC-02 | Rust              | Toàn bộ application core viết bằng Rust          |
| AC-03 | TUI               | Giao diện Ratatui, không cần GUI framework       |
| AC-04 | Hybrid Input      | Chuột và bàn phím hoạt động đồng thời            |
| AC-05 | SSH Config        | Tạo host dùng được bằng `ssh <Host>`             |
| AC-06 | Remote Terminal   | SSH và thao tác shell tương tác                  |
| AC-07 | Local Terminal    | Chạy local shell                                 |
| AC-08 | Multiple Sessions | Có thể quản lý nhiều terminal sessions           |
| AC-09 | File Browser      | Hiển thị Local/Remote                            |
| AC-10 | Mouse Navigation  | Click chọn file/folder và đường dẫn              |
| AC-11 | Multi-selection   | Chọn nhiều file/folder                           |
| AC-12 | Rename            | F2 mở Rename Dialog                              |
| AC-13 | Delete            | Del mở Delete Confirmation                       |
| AC-14 | Refresh           | F5 refresh panel hiện tại                        |
| AC-15 | Transfer          | Upload/download file và folder                   |
| AC-16 | Non-blocking      | UI không bị treo trong lúc transfer              |
| AC-17 | Security          | Không tự động bỏ qua SSH host key verification   |
| AC-18 | Recovery          | Xử lý được lỗi kết nối và file operation         |
| AC-19 | Cleanup           | Khôi phục terminal khi thoát                     |
| AC-20 | Mouse Transfer    | Internal drag-and-drop trên terminal tương thích |
| AC-21 | Docker Files      | Remote file operations và transfer qua `docker exec` |
| AC-22 | Config Workspace  | Chỉnh, lưu atomically và reload cấu hình từ TUI |
| AC-23 | Host Test         | Test kết nối là tùy chọn; kết quả hiển thị ở Host Details |

---

# 16. Final Architecture

```text
                     RUST SSH MANAGER
                            │
                            ▼
                   RATATUI INTERFACE
                            │
                 ┌──────────┴──────────┐
                 │                     │
                 ▼                     ▼
              KEYBOARD               MOUSE
                 │                     │
                 └──────────┬──────────┘
                            │
                            ▼
                      INPUT ROUTER
                            │
                            ▼
                     ACTION DISPATCHER
                            │
                            ▼
                       APP STATE
                            │
           ┌────────────────┼─────────────────┐
           │                │                 │
           ▼                ▼                 ▼
      HOST MANAGER     TERMINAL MANAGER   FILE MANAGER
           │                │                 │
           ▼                ▼                 ▼
      SSH CONFIG         PTY + SSH       LOCAL + SFTP
           │                │                 │
           │                │                 ▼
           │                │          TRANSFER MANAGER
           │                │                 │
           └────────────────┴─────────────────┘
                            │
                            ▼
                         OpenSSH
                            │
                            ▼
                       SSH SERVER
```

## Kết luận

Rust SSH Manager sẽ là một ứng dụng TUI chạy trực tiếp trên Ubuntu, sử dụng OpenSSH của hệ thống để quản lý kết nối và tương tác với server.

Ứng dụng có ba thành phần sử dụng chính: **Hosts, Terminal và Files**, cùng màn hình theo dõi **Transfers**.

File Manager sử dụng dual-pane Local/Remote, hỗ trợ chuột và bàn phím đồng thời. Các phím chức năng quan trọng được thống nhất:

**F2 = Rename · Del = Delete · F5 = Refresh**

Kiến trúc được chia thành các module độc lập, có hệ thống event/action chung, hỗ trợ các tác vụ bất đồng bộ và cho phép bổ sung tính năng mà không phải viết lại ứng dụng từ đầu.
