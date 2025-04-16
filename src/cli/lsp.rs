use std::borrow::Cow;

use dashmap::{DashMap, Map};
use fmt::format_document;
use log::debug;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::{ErrorCode, Result as LspResult};
use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidChangeWorkspaceFoldersParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingOptions, DocumentFormattingParams,
    DocumentRangeFormattingOptions, DocumentRangeFormattingParams, InitializeParams,
    InitializeResult, InitializedParams, OneOf, Position, PositionEncodingKind, ServerCapabilities,
    ServerInfo, TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, TextEdit, Url,
    VersionedTextDocumentIdentifier, WorkDoneProgressOptions, WorkspaceFolder,
    WorkspaceFoldersChangeEvent, WorkspaceFoldersServerCapabilities, WorkspaceServerCapabilities,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::opt;

mod fmt;

#[derive(Debug)]
struct Backend {
    opts: opt::Opt,
    #[allow(dead_code)]
    client: Client,
    document_map: DashMap<Url, String>,
    workspace_folders: RwLock<Vec<WorkspaceFolder>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        if let Some(new_folders) = params.workspace_folders {
            let mut folders = self.workspace_folders.write().await;
            *folders = new_folders
        }

        let supports_utf8 = params
            .capabilities
            .general
            .and_then(|general| general.position_encodings)
            .is_some_and(|position_encodings| {
                position_encodings.contains(&PositionEncodingKind::UTF8)
            });

        if !supports_utf8 {
            return LspResult::Err(tower_lsp::jsonrpc::Error {
                code: ErrorCode::InvalidParams,
                data: None,
                message: Cow::Borrowed("StyLua only supports UTF8 as position encoding"),
            });
        }

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "StyLua".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                position_encoding: Some(PositionEncodingKind::UTF8),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        ..Default::default()
                    },
                )),
                document_formatting_provider: Some(OneOf::Right(DocumentFormattingOptions {
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: Some(false),
                    },
                })),
                document_range_formatting_provider: Some(OneOf::Right(
                    DocumentRangeFormattingOptions {
                        work_done_progress_options: WorkDoneProgressOptions {
                            work_done_progress: Some(false),
                        },
                    },
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                ..ServerCapabilities::default()
            },
        })
    }
    async fn initialized(&self, _: InitializedParams) {
        debug!("initialized!");
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(
        &self,
        DidOpenTextDocumentParams {
            text_document:
                TextDocumentItem {
                    uri,
                    language_id: _,
                    version: _,
                    text,
                },
        }: DidOpenTextDocumentParams,
    ) {
        debug!("file opened");
        self.document_map.insert(uri, text);
    }

    async fn did_change(
        &self,
        DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version: _ },
            content_changes,
        }: DidChangeTextDocumentParams,
    ) {
        for TextDocumentContentChangeEvent {
            range,
            range_length: _,
            text,
        } in content_changes
        {
            match range {
                Some(range) => {
                    let mut document = self.document_map.get_mut(&uri).expect(
                        "`textDocument/didChange` was called so the document must be present",
                    );
                    let start = position_to_offset(range.start, &document)
                        .expect("Range must be in document");
                    let end = position_to_offset(range.end, &document)
                        .expect("Range must be in document");

                    let () = document.replace_range(start..end, &text);
                }
                None => {
                    self.document_map.insert(uri.clone(), text);
                }
            }
        }
    }

    async fn did_close(
        &self,
        DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
        }: DidCloseTextDocumentParams,
    ) {
        debug!("file closed!");
        self.document_map._remove(&uri);
    }

    async fn did_change_workspace_folders(
        &self,
        DidChangeWorkspaceFoldersParams {
            event: WorkspaceFoldersChangeEvent { added, removed },
        }: DidChangeWorkspaceFoldersParams,
    ) {
        debug!("workspace folders changed!");
        let mut folders = self.workspace_folders.write().await;
        let () = folders.retain(|folder| !removed.contains(folder));
        let () = folders.extend(added);
    }

    async fn formatting(
        &self,
        DocumentFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            options: format_options,
            work_done_progress_params: _,
        }: DocumentFormattingParams,
    ) -> LspResult<Option<Vec<TextEdit>>> {
        let edits = format_document(self, uri, None, format_options);
        LspResult::Ok(if edits.is_empty() { None } else { Some(edits) })
    }

    async fn range_formatting(
        &self,
        DocumentRangeFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            options: format_options,
            work_done_progress_params: _,
            range,
        }: DocumentRangeFormattingParams,
    ) -> LspResult<Option<Vec<TextEdit>>> {
        let edits = format_document(self, uri, Some(range), format_options);
        LspResult::Ok(if edits.is_empty() { None } else { Some(edits) })
    }
}

pub async fn start(opts: opt::Opt) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::build(|client| Backend {
        opts,
        client,
        document_map: DashMap::new(),
        workspace_folders: RwLock::new(vec![]),
    })
    .finish();

    Server::new(stdin, stdout, socket).serve(service).await;
}

fn position_to_offset(position: Position, s: &str) -> Option<usize> {
    s.split_inclusive('\n')
        .scan(0, |offset_at_start, line_with_newline| {
            let old_start = *offset_at_start;
            *offset_at_start += line_with_newline.len();
            Some(old_start)
        })
        .nth(position.line as usize)
        .map(|offset_at_start| s[0..offset_at_start + position.character as usize].len())
}
