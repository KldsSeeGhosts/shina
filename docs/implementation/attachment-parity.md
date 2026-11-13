# Attachment parity audit

Reference checkout: `/Volumes/DevDrive/AiStack/ksgcode`.
This records the reference rules before the remaining implementation changes.

## 1. Picker and intake

- `ComposerAttachmentStrip.tsx:702`: multiple files, clear input after selection,
  focus only if no chip insertion focused the editor.
- `ChatComposer.tsx:2647`: folders are path mentions, not uploaded directories.
  Reject a remote host with "Folders can only be referenced on a server running
  on this computer". Failed path lookup says "Couldn't get the path of ..."
  and "Type the folder path with @ instead." Insert at the end with a boundary.
- `ChatComposer.tsx:2683`: folders precede files on paste. Images, unsupported
  images, SVG and DNG claim a paste even with text. Other files claim only an
  otherwise empty paste. Paste-as-text skips image chips.
- `composerAttachmentFiles.ts:24`: infer GIF/JPEG/PNG/WebP only for empty or
  octet-stream MIME. HEIC/HEIF use MIME or a generic-MIME extension.
  SVG and DNG remain files. Supported provider image MIME types are images.
  Other image MIME types remain files only for workspace-preview extensions.
  Normalize DNG, SVG and generic video MIME types.
- `imageCompression.ts:15`: 50 MiB source bound, 64,000,000 HEIC pixel bound,
  1 MiB metadata scan, one HEIC preparation at a time. JPEG/PNG maximum edge
  2000. Quality ladder .92/.85/.78/.68, fallback scales 1/.75/.55.
  Prefer WebP when available, otherwise JPEG with white alpha matte.
  HEIC converts to JPEG. Preserve supported small original files. Conversion
  failures distinguish unreadable from too large after compression.

## 2. Limits

- `contracts/src/chatAttachment.ts:13`: 120,000 UTF-16 input characters,
  100 attachments, 10 MiB per image, 80 MiB total image bytes, 50 MiB per file.
  File minimum 1 byte. GIF/JPEG/PNG/WebP only as provider images.
  Image data URLs are at most 14,000,000 characters. Names are at most 255
  characters, MIME at most 100, attachment IDs at most 128 and alphanumeric,
  underscore or hyphen in KSG's wire format.
- `chatAttachment.ts:208`: total counts include image-MIME files. Exact
  aggregate errors mention "per message or question response".
- `useComposerAttachmentIntake.ts:161`: reserve slots synchronously, continue
  accepting valid files in a mixed batch, last error wins. Empty files and
  oversized files have separate errors. Unknown file capability permits local
  staging; known absent capability rejects files.
- `composerAttachmentFiles.ts:124`: unknown capability blocks sending with
  "Waiting for the server before file attachments can send"; known absent
  capability asks to remove files. Enforce a server's advertised byte limit.

## 3. Staged uploads

- `attachmentUploadQueue.ts:33`: three concurrent transfers per environment,
  five-minute timeout. Mint a pending upload, transfer bytes with progress,
  finalize, verify restored uploads, stamp the owning draft even while inactive.
- `attachmentUploadState.ts:26`: pending uploads block send, with singular/plural
  wording. Failures allow Send to retry. Floor finite, clamped percentages.
- `useAttachmentUploadProgress.ts`: delay percentage display by 150 ms.
- `ComposerAttachmentStrip.tsx:281`: same image tile identity during preparation,
  "Preparing", progress and a retry action with the error as its tooltip.
  File/video rows have progress and retry too. Removal cancels work.
- `composerAttachmentFiles.ts:157`: capability loss releases only unpersisted
  uploads. Keep hydrated file upload ownership. Reconnection retries only the
  failed job still owning the file. Cancellation must not delete a claimed blob.
- `attachmentUploadQueue.ts:571`: pending uploads expire after 24 hours.

## 4. Needs reattach

- `composerDraftStore.ts:181`: a file without local bytes or uploaded identity
  needs reattach. `:831` matches exact name and byte count, exact MIME, normalized
  DNG aliases, or generic marker MIME for videos.
- `useComposerAttachmentIntake.ts:161`: a matching marker uses no additional
  slot, each marker matches at most once in a batch.
- `ComposerAttachmentStrip.tsx:430`: disable preview of markers. Say "Attach
  again" when the server can accept the file, otherwise "Remove to send".

## 5. Large paste

- `client-runtime/src/textPaste.ts:1`: 32 KiB UTF-8 threshold, or the paste
  would exceed the 120,000 UTF-16 prompt limit after replacing the selection.
  Empty paste and Mod+Shift+V stay inline.
- `textPaste.ts:47`: case-insensitive unique `pasted-text.txt`, then
  `pasted-text-2.txt`, etc. Reserve names for the lifetime of the active target.
- `useComposerAttachmentIntake.ts:359`: fold into a UTF-8 file with an inline
  chip and pasted-text source. If staging is unavailable, fall back inline
  unless the prompt limit would be exceeded. In that case consume the paste
  and report "Pasted text is too large for this message" plus removal guidance.
  If the file exceeds the staging limit, fall back inline unless the prompt
  limit would be exceeded, then report "Pasted text is too large to attach".
  Success notice names the file, size and paste-as-text shortcut.

## 6. Previews

- `ComposerAttachmentStrip.tsx:195`: click prepared images to expand the image
  collection at that index. Preparing images cannot expand. Resting thumbnails
  expand the same collection.
- `ExpandedImagePreview.tsx:192`: videos expand alone, own temporary URLs, and
  release them on close. Uploaded videos resolve from their owning environment.
- `ComposerAttachmentStrip.tsx:532`: file preview dialog, Draft origin, remove
  and close controls, maximum width 56 rem, height min(85vh,52rem).
- `files/AttachmentFilePreview.tsx:107`: kind-specific file viewers, download,
  copy, wrap, source/rendered toggle and close. Includes images, video, audio,
  PDF, text, markdown, HTML, Mermaid, delimited tables, office and fonts.
  Remote assets retry authorization after five minutes. Text reads are bounded.
  Failures have retry. Sent messages open the same file/media viewers.

## Implementation and verification

Items 1 and 2 arrived committed as `1018ecb` and `2329f65`. Their original
pre-commit test logs were not in the handoff. The final-tree checks exercise
their code again.

Paths in the following tables are relative to each repository. KSG web paths
start at `apps/web/src/`; contract paths start at `packages/contracts/src/`.
The rule inventory above and the mappings below must be read with the stopped
and incomplete rules below. This is not a claim of complete preview parity.

## Rule-to-function mapping

| Item | KSG source and rule | Shina implementation |
| --- | --- | --- |
| 1 | `components/chat/ComposerAttachmentStrip.tsx:702`, multiple files, clear selection, refocus if no chip | `attachment_picker::open`; `ui::pick_attachment`; `State::attach_paths_on_to` captures machine and thread before the dialog |
| 1 | `components/chat/folderDrop.ts:7`, primary local or direct saved loopback only | `State::attach_paths_to`; `State::attach_paths_on_to`, loopback URL/IP checks |
| 1 | `components/chat/folderDrop.ts:43` and `ChatComposer.tsx:2647`, path errors, folder mention at end with a leading boundary | `State::attach_paths_to`; `composer_tokens::Token::File`; no folder upload |
| 1 | `components/chat/ChatComposer.tsx:2683`, folders before files, captioned paste ownership, bypass image chips | `State::paste_composer_content`; `attachment_intake::claims_paste`; `attach_image_with_options` |
| 1 | `components/chat/composerAttachmentFiles.ts:53`, generic MIME inference | `attachment_intake::native_mime`, `classify`, `file_mime` |
| 1 | `composerAttachmentFiles.ts:74`, DNG/SVG remain files, normalize DNG/SVG/video MIME | `attachment_intake::file_mime`, `attachments::attach_file` |
| 1 | `composerAttachmentFiles.ts:84`, GIF/HEIC/HEIF/JPEG/PNG/WebP images, preview formats as files, unsupported-image error | `attachment_intake::classify`, `Failure::message`, `attachments::attach_file` |
| 1 | `lib/imageCompression.ts:14`, 50 MiB source bound, HEIC 64 million pixels and 1 MiB metadata scan | `attachment_intake::MAX_SOURCE_BYTES`, `validate_heic_dimensions`, `decode_heic` |
| 1 | `imageCompression.ts:430`, 2000 px JPEG/PNG fit, preserve small supported originals, quality and scale ladders, white JPEG matte, data-URL budget | `attachment_intake::fit_size`, `prepare`, `jpeg`; EXIF orientation applies before re-encoding |
| 1 | `imageCompression.ts:542`, HEIC preparation and JPEG conversion | macOS `decode_heic` uses `NSBitmapImageRep`, then `prepare`; Windows gap below |
| 2 | `chatAttachment.ts:13`, 120,000 UTF-16 prompt units | `composer::prompt_length_validation_message`; `composer_paste::would_exceed_limit` |
| 2 | `chatAttachment.ts:14`, 100 attachments | `attachments::MAX_ATTACHMENTS`, `validate_prompt_images`; `Queue::enqueue_on` reserves preparing-image slots |
| 2 | `chatAttachment.ts:15`, 10 MiB image limit, four supported provider MIME types | `attachment_intake::prepare`; `attachments::attach_bytes`, `read`, `validate_prompt_images` |
| 2 | `chatAttachment.ts:16`, 80 MiB aggregate image bytes, including supported image MIME files | `attachments::validate_prompt_images`, saturating sum |
| 2 | `chatAttachment.ts:17`, nonempty files up to 50 MiB, metadata length checks | `attachments::validate_file_metadata`, `attach_file`, `store_uploaded_file`, transfer/upload validation |
| 2 | `chatAttachment.ts:208`, exact aggregate error wording | `attachments::validate_prompt_images` |
| 2 | `useComposerAttachmentIntake.ts:161`, continue mixed batches and keep the last error | `State::attach_paths_to`; `Queue::enqueue_on`; late preparation success does not clear another error |
| 2 | `composerAttachmentFiles.ts:124`, optimistic staging while capability unknown, block sending until known, reject known unsupported files | `State::file_attachment_staging_supported_on`, `attachment_send_block_reason`; `Capability::{Unknown,Legacy,Staged}` |
| 2 | `composerAttachmentFiles.ts:120,143` and `client-runtime/state/attachments.ts:192`, clamp a server's advertised file bound, exact KB/MB/byte error units, recheck existing drafts | `protocol::{file_attachment_limit,file_attachment_too_large}`; `State::file_attachment_staging_limit_on`, synchronous/queued intake, `fold_paste`, `Queue::poll`, `attachment_send_block_reason` |
| 2 | Provider-specific acceptance | `model` input-modality guards; `codex::codex_input`; `claude::input_blocks_with_budget`; `claude_attachment_budget::Budget`; `attachments::acp_parts` |
| 3 | `lib/attachmentUploadQueue.ts:33`, three transfers per environment and five-minute upload timeout | `attachment_uploads::MAX_UPLOADS_PER_ENVIRONMENT`, `UPLOAD_TIMEOUT`, registry semaphore, `upload_cycle` |
| 3 | `useComposerAttachmentIntake.ts:225`, preparing placeholders, reserve before awaits, remove cancelled results | `Queue::enqueue_on`, `Queue::poll`, `Queue::remove`; background image/file preparation |
| 3 | `imageCompression.ts:216`, shared source thumbnails and inheritance | `prepare_with_thumbnail`, `publish_thumbnail`; `Event::Thumbnail` is nonterminal; `Entry::thumbnail` retains one handle for preparing, ready and resting tiles |
| 3 | `lib/attachmentUploadState.ts:26`, singular/plural send blocking; failure allows Send retry | `Phase::blocks_send`, `Queue::block_reason`, `State::attachment_send_block_reason`; transport retry in `client::uploaded_command_capabilities` |
| 3 | `lib/attachmentUploadState.ts:48` and `hooks/useAttachmentUploadProgress.ts:9`, bounded/floored progress after 150 ms | `progress_label`, `Phase::label`; `ui::attachment_upload_status`, `attachment_upload_file_rows` |
| 3 | `ComposerAttachmentStrip.tsx:281`, retry with reason, removal while preparing/uploading | `State::retry_attachment_upload`, `Queue::retry`, `request_remove_attachment`, `remove_attachment`; existing UI helpers |
| 3 | `composerAttachmentFiles.ts:157`, capability lost mid-upload, retain persisted file ownership | `Queue::poll`, `Entry::previous`, cancellation token and lease release |
| 3 | `attachmentUploadQueue.ts:401`, owner resolution after draft moves, inactive completion persistence | `target_mut`, `persist_cached_draft`, `Queue::poll`, origin-environment picker result |
| 3 | `attachmentUploadQueue.ts:467`, reconnect retry only the owning failed job | `Queue::poll` connection transitions and attempt generations; authentication denials require manual credential repair |
| 3 | `attachmentUploadQueue.ts:571`, restored-upload verification, 24-hour pending expiry | `verify_upload`; `staged_uploads::{read,publish,sweep}` |
| 3 | server `attachmentStore.ts:30`, one-hour partial retention, pending claim does not delete shared/claimed bytes | staged upload expiry in `server::upload_start`; `staged_uploads::{claim,release}` |
| 4 | `composerDraftStore.ts:181`, file with neither bytes nor upload identity needs reattach | `Attachment::needs_reattach`; `from_value` retains missing ordinary-file references |
| 4 | `composerDraftStore.ts:831`, exact name/size/MIME, DNG aliases, generic video markers | `Attachment::matches_reattach` |
| 4 | `useComposerAttachmentIntake.ts:161`, matching marker takes no new slot and is used at most once | `Queue::enqueue_on`; synchronous `State::attach_paths_to`; preserved logical ID and existing inline chip |
| 4 | `ComposerAttachmentStrip.tsx:430`, disabled marker preview, Attach again/Remove to send, remove marker | `ui::attachment_upload_file_rows`; `State::preview_attachments` rejects markers; send guard |
| 4 | `attachmentUploadQueue.ts:197`, expired persisted file with no bytes becomes a marker | `Queue::poll` verification completion clears the unavailable upload stamp |
| 5 | `textPaste.ts:31`, empty/bypassed text stays inline; 32 KiB UTF-8 threshold or prompt-limit trigger | `State::paste_composer_content`; `composer_paste::{should_fold,would_exceed_limit}`; `State::fold_paste` |
| 5 | `textPaste.ts:47`, unique names, case-insensitive, reserve removed names until target changes | `composer_paste::next_name`; `State::pasted_text_names` |
| 5 | `textPaste.ts:73`, replace the selected range before counting prompt units | `composer_paste::would_exceed_limit`; `Thread::insert_attachment_chips` |
| 5 | `components/ComposerPromptEditorTiptap.tsx:1020` and `composer-rich-text-doc.ts:1803`, count serialized Markdown with mapped selection boundaries, not display text | `composer_rich::{markdown_mapped,inline_mapped,markdown_selection}`; `composer_paste::would_exceed_limit` |
| 5 | `useComposerAttachmentIntake.ts:389`, capability/capacity/busy fallback, consume overflowing paste with exact error/guidance | `State::fold_paste` |
| 5 | `useComposerAttachmentIntake.ts:420`, file-size fallback and distinct overflow error | `State::fold_paste`; 50 MiB preflight |
| 5 | `useComposerAttachmentIntake.ts:211`, success notice, byte size, platform shortcut | `State::fold_paste`, `push_toast` |
| 5 | `chatAttachment.ts:158`, pasted-text source remains path-only | `AttachmentSource::PastedText`; `stage_transfer_file` preserves source; `acp_parts` avoids embedding; Claude skips pasted-source native PDF blocks |
| 6 | `ComposerAttachmentStrip.tsx:195`, expand prepared image; preparing tile cannot expand | `ui::attachment_chip`, `image_preview`, `preparing_attachment_chip`; `State::preview_attachments` |
| 6 | `ComposerAttachmentStrip.tsx:584`, resting thumbnails open the collection | resting `image_preview` handlers in `ui::composer` |
| 6 | `ExpandedImagePreview.tsx:192`, image-only gallery, selected index, video alone | `State::preview_attachments`, `navigate_attachment_preview` |
| 6 | `ExpandedImageDialog.tsx:105`, previous/next media, close, title/count | `attachment_preview::view`; arrow-key navigation; existing keys retained |
| 6 | `chat/ZoomableImage.tsx:9, 101, 139, 172`, maximum 8x zoom, pointer anchors, native pinch/Ctrl-wheel, drag threshold 4 px, keyboard pan 40 px, plus/minus factor 1.5, zero to fit, resize refit | `Preview::change_zoom`, `zoom_offset`, image viewport gesture/layout handlers and modal key handler; Rune `on_wheel`, `on_magnify`, `on_drag`, `on_scroll_offset` and `Tx::scroll_to_instant` |
| 6 | `ComposerAttachmentStrip.tsx:357`, paused muted video poster and play overlay | `ui::image_preview`, native poster cache; `videoFirstFrame.ts:1` seek rule |
| 6 | `ExpandedImageDialog.tsx:61`, expanded video autoplay; owned source lifetime | `Preview::load`, Rune `Player`, drop on close/route change |
| 6 | `media/MediaVideoPlayer.tsx:134`, playback error and retry | `attachment_preview::view`; native Player retry, play/pause/seek/mute/volume/rate/full-screen controls |
| 6 | `files/AttachmentFilePreview.tsx:107`, file preview, Draft origin, save/remove/close | `attachment_preview::view`; Rune save dialog; existing `html_window::Windows` for native documents |
| 6 | `AttachmentFilePreview.tsx:169`, missing asset/retry and remote ownership | `download_staged`, `cache_files`; preview completion checks target and selected logical ID |
| 6 | `shared/filePreview.ts:171`, strict UTF-8, binary rejection, bounded small preview | `attachment_text_preview::decode_small`; `Preview::load` |
| 6 | `files/largeTextProtocol.ts:1`, 256 MiB, five million rows, 1024 columns, 32-row indexing semantics | `attachment_text_preview::Document`, `attachment_preview::view` virtual rows |
| 6 | `largeText.worker.ts:99`, CR/LF, BOM, CSV quotes, NUL and 1 MiB record bound | `Document::index`, `Document::row` |
| 6 | `LargeTextViewer.tsx:9`, first 16 KiB before background indexing | `Preview::load`; `State::load_attachment_text_document`; cancellation token |
| 6 | `files/binaryPreview.worker.ts:184`, read-only SQLite, 100 tables, exclude virtual/internal tables | `attachment_sqlite_preview::Database::open` |
| 6 | `binaryPreview.worker.ts:150`, 50-row pages, 50 columns, 1024 characters, blob labels, NULL | `Database::page`; table selection and previous/next controls |
| 6 | `files/FontViewer.tsx:7`, 10 MiB font, FontFace validation, 30-second timeout, specimen sizes | `html_window::font_document`, `assets/attachment-font-preview.js`; isolated platform engine |
| 6 | `AttachmentFilePreview.tsx:338`, PDF and rendered HTML | `Windows::open_attachment_async`; WKWebView PDF data loading and isolated WebView2 PDF document |
| 6 | `shared/filePreview.ts:33`, generic-MIME inference, MIME precedence, complete text/basename and audio-extension lists | `attachment_preview::kind`; definite image MIME is not overridden by a `.db` suffix |
| 6 | `files/binaryPreview.worker.ts:21,43,208`, gzip expansion, TAR header checks, GNU long names, PAX metadata, archive member reads | `attachment_archive_preview::{Archive::parse,Archive::entry,tar,read_bounded}`; no member is extracted to a filesystem path |
| 6 | `client-runtime/zip/boundedZip.ts:4` and `zipDirectory.ts:1`, 50 MiB expanded bytes, 10 MiB member previews, 2048 entries, ratio 200, 1024-character paths, 8 MiB XML parts, ZIP64 directory preflight | `attachment_archive_preview::{zip,u16_at,u32_at,u64_at}`; streaming stored/deflate decoding, full central-directory preflight |
| 6 | `files/BinaryViewer.tsx:181,198,305`, archive folders, parent/back navigation, byte counts, source-only text/image members | `attachment_preview::{archive_view,State::open_archive_entry,State::load_attachment_archive}`; background parsing and owner-generation checks |

## Protocol and older builds

Protocol remains major 1, minor 11. New optional capabilities:

- `staged-attachments-v1`: mint, page upload, commit, verify, fetch pending
  bytes, release and claim a draft-owned lease. Unknown peers wait. Older peers
  use the existing `image-uploads`/`file-attachments-v1` send-time path. A
  capability lost during transfer cancels it without discarding the draft.
  A pending-only restored file cannot use legacy send-time upload without local
  bytes. It retains its lease and shows an explicit update/remove/reattach guard.
- `attachment-limits-v1`: identifies the 80 MiB aggregate image allowance and
  removal of the eight-image ceiling. An older peer retains eight images and
  20 MiB aggregate; the client blocks larger batches before uploading them.
  The hello response also publishes `limits.fileAttachmentBytes` under this
  capability. A smaller numeric value bounds ordinary-file intake, pending
  transfers, paste folding and existing drafts. Explicit null disables files.
  A missing field, including an earlier build with this capability, explicitly
  retains the original 50 MiB file contract. Values above 50 MiB clamp to 50 MiB.
- `pasted-text-attachments-v1`: preserves the optional `_tag: pasted-text`
  source. Older readers receive an ordinary path-backed file without the new
  field. Pi/Codex/Claude already keep ordinary text files path-only. For ACP,
  a known older peer must not silently embed folded text. Folding falls back
  inline when safe, and an already-folded ACP draft blocks with an update
  message after capability loss.

New metadata (`asset_id`, `uploaded`, `source`) is local draft state or
capability-gated transport metadata. Logical IDs survive preparation and
reattachment; immutable blob IDs replace them at the transport boundary.
The SHA-256 blob store and existing Shina wire ID format remain intact.

## Per-provider findings

| Harness | Acceptance found in Shina/KSG source | Result |
| --- | --- | --- |
| Pi | Model must declare image input. Typed base64 image parts plus path-only ordinary files. KSG Pi RPC allows the expanded image record, not an 8 MiB record cap. No additional numeric model-specific limit is advertised. | Shared 100 / 10 MiB / 80 MiB / 50 MiB validation; RPC record budget expanded to admit the image payload and duplicated MCP image content. |
| Codex | `codex_input` uses image data URLs and absolute file-path text. Catalog input modalities govern image acceptance. No further numeric attachment quota appears in the local app-server adapter. | Shared limits and modality checks; do not claim that every upstream model accepts every allowed batch. |
| Claude | KSG `ClaudeAdapterV2.ts:2704` reserves 28 MiB retained encoded content below the 32 MB request envelope, across committed and pending usage. 100 PDF pages, 20 MiB native PDF read cap, GIF/JPEG/PNG/WebP images. | `claude_attachment_budget` tracks pending/committed reservations, failures/retries, compaction and restored history. Images/PDFs that do not fit remain available by path. Pasted text is never promoted to a native PDF. |
| ACP | Generic ACP negotiates image input and embedded context, not numeric byte/count limits. Shina embeds ordinary UTF-8 resources only up to its existing 256 KiB text-read bound, otherwise links/paths. KSG's Antigravity specialization separately has 1 MiB native text, 20 MiB audio/video and 50 MiB native aggregate limits, with agent-version/model qualification. | Image capability guard, bounded generic resources and path fallback remain. Pasted-text resources are path-only. Antigravity's specialized native audio/video adapter is not implemented by Shina's generic ACP adapter; it does not send typed native audio/video payloads. Live agent-specific numeric acceptance is unverified. |

## Stopped and incomplete rules

Complete KSG parity is **not** established. In particular, item 6 is partial.
Item 1 also retains the permitted Windows HEIC/HEIF gap.

- Windows HEIC/HEIF preparation is not implemented. `decode_heic` on Windows
  returns `Unreadable`. The locked `image` codecs and Rune decoder contain no
  HEIC decoder. macOS uses the already-linked AppKit/ImageIO-backed
  `NSBitmapImageRep`. An optional Windows HEIF codec is not bundled or tested.
- Compression uses KSG's JPEG fallback because locked `image` exposes lossless
  WebP encoding, not the quality-controlled lossy WebP encoder needed for KSG's
  quality ladder. This does not reproduce the output bytes of a KSG browser
  with working lossy WebP canvas support. Small supported originals remain intact.
- KSG's Office and EPUB viewers are not ported. Their
  browser decoder/parser packages are not in Shina's locked dependency tree.
  Rune's image/player APIs and the linked native webview do not themselves
  lay out Office documents with KSG's parser, paging and safety rules. The native host cannot
  reproduce those viewers without their parsers or a separate implementation.
  Such files retain the explicit "No preview for this file" and Save fallback.
  This is an unimplemented part of item 6, not a claim that the formats are
  impossible to support in Shina.
- The missing rich-document rules include the following KSG implementations.
  They have no corresponding Shina viewer function yet:
  - `files/officePreviewProtocol.ts:5`: 50 MiB input, 250,000 workbook rows,
    1024 workbook columns.
  - `files/officeArchive.ts:14`: Word body 4 MiB/50,000 elements/1,000,000 tags;
    per-element 100,000 tags; paragraph 128 KiB; 8192 styles; styles 16 MiB,
    400,000 units, 131,072 visits and 2,000,000 property visits; numbering
    20,000 references/32 MiB/400,000 markup; auxiliary parts 8 MiB/1,000,000
    markup; expanded document 64 MiB/4,000,000 markup. The worker preserves
    section properties and trims before passing content to BetterOffice.
  - `files/extendedDocumentPreview.ts:12,299`: ODS 500 rows and 50 columns,
    XML 8,388,608 UTF-16 units, depth 100, 100,000 nodes, rendered markup
    16,777,216 UTF-16 units; OpenDocument
    page/sheet navigation and EPUB package/spine/XHTML handling.
  - `files/extendedDocumentPreview.ts:466`: RTF 8 MiB input and
    8,388,608 UTF-16 units of rendered content,
    500 pages, nesting 100, byte/code-page decoding, binary destinations,
    Unicode fallback, formatting and ignored non-content destinations.
- The notebook viewer is ported: `attachment_notebook_preview::parse`
  reproduces `files/extendedDocumentPreview.ts:426` — strict UTF-8, notebook
  v4 only, 8 MiB input, 500 cells, 256,000 UTF-16 units per source/text
  output, 100 outputs/cell, image base64 text 4,194,304 UTF-16 units,
  16,777,216 pixels, and KSG's error messages. `attachment_preview`'s
  notebook branch reproduces `NotebookPreview.tsx` — twenty cells at a
  time, inert Markdown, stream/result/display/error saved outputs with
  ANSI escapes stripped from tracebacks, and nothing executed. One
  difference: KSG mounts HTML outputs in bounded browser frames; Shina
  has no embedded browser, so an HTML output shows its `text/plain`
  alternative, or the muted line "HTML output is not shown" when it has
  none.
- Full standalone Mermaid parity is not ported. The pinned Rune checkout has
  an optional native Mermaid parser, but its documented behavior skips lines
  outside its supported subset. That is not KSG's renderer/error behavior.
  Shina does not enable it as an approximate replacement for the KSG renderer.
  The concrete Rune reference is `crates/rune-kit/src/mermaid.rs:17` in the
  pinned checkout. Its documentation explicitly says unreadable lines are skipped.
- The previous audit incorrectly said Rune had no public wheel/pinch callbacks.
  Its `input/canvas_input.rs` has both. Image expansion now uses those APIs,
  with pointer anchors, dragging, keyboard panning, resize refit and KSG's zoom
  constants. Full native platform gesture QA and exact media-dialog geometry
  remain unverified.
- The virtual CSV/text viewer uses Rune virtual rows. It does not yet match
  KSG's sticky measured header, horizontal column virtualization and table
  measurement exactly. Small source previews do not yet use KSG-equivalent
  syntax highlighting.
- Archive previews now use the already-locked `flate2` 1.1.11 and `crc32fast`
  1.5.4 as direct dependencies. ZIP/JAR, TAR and gzip/TGZ are implemented with
  bounded decoding, ZIP64 directory checks, GNU long names and PAX metadata.
  Archive source highlighting is part of the source-viewer gap above.
  Unicode locale-aware filename ordering and localized date presentation do
  not yet match the browser's `localeCompare`/`toLocaleDateString`. Malformed
  ZIP codec errors have not been exhaustively compared with JSZip.
- PDF, rendered HTML and fonts use separate owned native document windows,
  because Rune has no embedded browser element. They are not embedded inside
  the Rune dialog. Video uses Rune directly on both platforms.
- KSG `MediaVideoPlayer.tsx:166` uses browser-native video controls. The Rune
  viewer has custom controls. Play/pause, seeking, mute, volume, rate and
  full-screen operations exist, but the volume/rate button cycles are not a
  port of browser-native sliders/menus. That control behavior remains a parity
  gap, not a Rune impossibility.
- Question-attachment guards and questionnaire UI remain owned by the
  parallel approvals/questions worktree. This work did not edit those panels
  or `State::send`.

These entries are recorded as gaps, not silently treated as out of scope.
The first two commits also predate this continuation; their original
test-first/pre-commit sequence cannot be independently reconstructed here.

## Tests and verification record

Test-first failure logs for this continuation:

- `/tmp/shina-attachments-item4-red.log`, missing reattach implementation.
- `/tmp/shina-attachments-item5-red.log`, missing UTF-16 prompt-limit and name
  helpers.
- `/tmp/shina-attachments-item6-red.log`, missing preview classification.
- `/tmp/shina-attachments-item6-layout-red.log`, layout/action failures.
- `/tmp/shina-attachments-sqlite-red.log`, missing SQLite implementation.
- `/tmp/shina-attachments-zoom-red.log`, missing pointer-anchor/gesture implementation.
- `/tmp/shina-attachments-rich-paste-red.log`, rich formatting omitted from the prompt-limit count.
- `/tmp/shina-attachments-preview-types-red.log`, missing basename/audio/MIME classification.
- `/tmp/shina-attachments-archive-red.log`, missing archive parser.
- `/tmp/shina-attachments-zip-duplicates-red.log`, duplicate members incorrectly
  charged before KSG's last-entry-wins resolution.
- `/tmp/shina-attachments-advertised-limit-red.log`, advertised smaller file
  bounds were not enforced by intake.

Unit coverage includes classification, image preparation bounds, metadata,
limits, capability states, preparation reservation/removal, capability loss
and regain, progress delay/clamping, inactive origin completion, auth denial,
reattach matching/restoration at capacity, paste thresholds/selection/UTF-16,
name reservations, source negotiation, ACP lazy-paste behavior, SQLite bounds,
strict text/CSV indexing, source-thumbnail inheritance, and native document input
isolation. Matching-marker tests select a different path with the same metadata
and verify restoration, rather than merely recreating the original path.
Archive tests cover stored/deflated ZIP members, traversal names without disk
extraction, central-directory rejection, gzip limits, TAR checksums and bounded
member reads. Preparation placeholders retain one source-thumbnail handle
through the swap, with an icon fallback when thumbnail generation fails.

Headless layout tests in `src/ui.rs` use 620, 960, 1280 and 1920 px for preparing,
progress, retry, marker rows, image expansion, ordinary text, video playback,
CSV, SQLite and native-document launch controls. A generated one-second
64-by-48 H.264 fixture exercises silent headless decoding and playback time.
Archive folder/member/back controls and pinch/keyboard image panning also run
at all four widths.

`tests/server_runtime.rs::staged_files_linked_servers_failure_retry_claim_and_older_peer_negotiation`
uses two real linked test servers, a failed upload, retry, pending-blob paging,
claim/release, and an older peer. It also exercises image hash failure/retry,
identical staged PNGs on both servers, pending-image byte reads/releases, and
legacy image upload. The fixture model intentionally lacks vision, so image
provider dispatch is covered separately by modality guard tests, not by
pretending that fixture supports images. All profiles are temporary.

The frontend-design guidance kept the existing Pixel Voyage palette, typography,
helpers and keys. File rows follow KSG, and video tiles remain in the media strip
even when their files have inline chips.

An existing peer-delegation integration test twice failed under the default
67-way integration concurrency because its target had not advertised its
fixture model. It passed alone and in the four-thread integration runs.
No production service or unrelated source was changed to hide that failure.

Final checks on the implementation tree:

- `cargo fmt --check` passed.
- `cargo build` passed.
- `cargo test -- --test-threads=4` passed with 1,070 unit tests, 11 ignored,
  and 68 integration-target tests.
- `node --check assets/attachment-font-preview.js` passed.
- Three additional consecutive `cargo test --test server_runtime --
  --test-threads=4` runs passed, 68 tests each, in 59.82, 60.59 and 66.21 seconds.
  Logs are `/tmp/shina-attachments-verified-integration-{1,2,3}.log`; the full
  command log is `/tmp/shina-attachments-verified-all.log` and the build log is
  `/tmp/shina-attachments-verified-build.log`.

The preview commit also contains cross-item corrections found during
verification: advertised file bounds, source-thumbnail inheritance, immutable
reattach identities, lazy-paste source preservation, and serialized rich-text
paste counting. It does not claim completion of the missing viewers above.
The final handoff records its hash and confirms deletion of this worktree's
build directory after all checks and the commit.

Native Windows execution, platform PDF/font rendering, real HEIC fixtures and
live provider acceptance remain unverified. The original test-first sequence
for the first two inherited commits remains unverified. No CPA calls or
production-profile/app changes are part of verification. Unit tests use
thread-owned temporary profiles; server fixtures explicitly set temporary
`XDG_DATA_HOME` values. The final check command also sets a temporary
`XDG_DATA_HOME` for its parent process.
