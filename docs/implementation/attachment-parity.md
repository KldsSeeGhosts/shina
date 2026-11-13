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
- `chatAttachment.ts:285`: total counts include image-MIME files. Exact
  aggregate errors mention "per message or question response".
- `useComposerAttachmentIntake.ts:161`: reserve slots synchronously, continue
  accepting valid files in a mixed batch, last error wins. Empty files and
  oversized files have separate errors. Unknown file capability permits local
  staging; known absent capability rejects files.
- `composerAttachmentFiles.ts:116`: unknown capability blocks sending with
  "Waiting for the server before file attachments can send"; known absent
  capability asks to remove files. Enforce a server's advertised byte limit.

## 3. Staged uploads

- `attachmentUploadQueue.ts:33`: three concurrent transfers per environment,
  five-minute timeout. Mint a pending upload, transfer bytes with progress,
  finalize, verify restored uploads, stamp the owning draft even while inactive.
- `attachmentUploadState.ts:24`: pending uploads block send, with singular/plural
  wording. Failures allow Send to retry. Floor finite, clamped percentages.
- `useAttachmentUploadProgress.ts`: delay percentage display by 150 ms.
- `ComposerAttachmentStrip.tsx:281`: same image tile identity during preparation,
  "Preparing", progress and a retry action with the error as its tooltip.
  File/video rows have progress and retry too. Removal cancels work.
- `composerAttachmentFiles.ts:144`: capability loss releases only unpersisted
  uploads. Keep hydrated file upload ownership. Reconnection retries only the
  failed job still owning the file. Cancellation must not delete a claimed blob.
- `attachmentUploadQueue.ts:571`: pending uploads expire after 24 hours.

## 4. Needs reattach

- `composerDraftStore.ts:181`: a file without local bytes or uploaded identity
  needs reattach. `:831` matches exact name and byte count, exact MIME, normalized
  DNG aliases, or generic marker MIME for videos.
- `useComposerAttachmentIntake.ts:161`: a matching marker uses no additional
  slot, each marker matches at most once in a batch.
- `ComposerAttachmentStrip.tsx:463`: disable preview of markers. Say "Attach
  again" when the server can accept the file, otherwise "Remove to send".

## 5. Large paste

- `client-runtime/src/textPaste.ts:1`: 32 KiB UTF-8 threshold, or the paste
  would exceed the 120,000 UTF-16 prompt limit after replacing the selection.
  Empty paste and Mod+Shift+V stay inline.
- `textPaste.ts:43`: case-insensitive unique `pasted-text.txt`, then
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

Items 1 and 2 arrived committed as `1018ecb` and `2329f65`.
The following sections will be completed with implementation mappings,
verification results and explicit gaps.
