"""Fast, persistent Android Storage Access Framework integration.

The user grants one root folder once. All later navigation happens in the
app's own browser, while file copies and exports run on background threads.
"""
import mimetypes
import os
import tempfile

from kivy.clock import Clock, mainthread


class AndroidStorage:
    """SAF adapter; instantiate only on Android."""

    REQUEST_CODE_BASE = 5100
    RESULT_OK = -1
    IMAGE_EXTENSIONS = (".png", ".jpg", ".jpeg", ".webp", ".bmp")

    def __init__(self, app_ref):
        from android import activity as android_activity
        from jnius import autoclass

        self.app_ref = app_ref
        self.Intent = autoclass("android.content.Intent")
        self.Uri = autoclass("android.net.Uri")
        self.DocumentsContract = autoclass("android.provider.DocumentsContract")
        self.Document = autoclass("android.provider.DocumentsContract$Document")
        self.OpenableColumns = autoclass("android.provider.OpenableColumns")
        self.activity = autoclass("org.kivy.android.PythonActivity").mActivity
        self.resolver = self.activity.getContentResolver()
        self.directory_mime = str(self.Document.MIME_TYPE_DIR)
        self._pending = {}
        self._output_dirs = {}
        self._output_files = {}
        self._next_request_code = self.REQUEST_CODE_BASE

        self.cache_root = os.path.join(app_ref.user_data_dir, "radar_saf_cache")
        os.makedirs(self.cache_root, exist_ok=True)
        self._root_state_path = os.path.join(
            app_ref.user_data_dir, "radar_storage_root.txt"
        )
        self.tree_uri = None
        self.root_document_id = None
        self._load_saved_root()

        self._android_activity_module = android_activity
        android_activity.bind(on_activity_result=self._on_activity_result)

    def _load_saved_root(self):
        try:
            with open(self._root_state_path, "r", encoding="utf-8") as source:
                uri_text = source.read().strip()
            if not uri_text:
                return
            uri = self.Uri.parse(uri_text)
            root_id = str(self.DocumentsContract.getTreeDocumentId(uri))
            self.tree_uri = uri
            self.root_document_id = root_id
        except Exception:
            self.tree_uri = None
            self.root_document_id = None

    def ensure_root_access(self, callback):
        """Use the saved SAF grant, or request one on first launch."""
        if self.tree_uri is not None and self.root_document_id:
            Clock.schedule_once(lambda _dt: callback(True), 0)
        else:
            self.pick_root(callback)

    def pick_root(self, callback):
        intent = self.Intent(self.Intent.ACTION_OPEN_DOCUMENT_TREE)
        intent.addFlags(
            self.Intent.FLAG_GRANT_READ_URI_PERMISSION
            | self.Intent.FLAG_GRANT_WRITE_URI_PERMISSION
            | self.Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
        )
        self._launch(intent, {"kind": "root", "callback": callback})

    def _launch(self, intent, request):
        code = self._next_request_code
        self._next_request_code += 1
        if self._next_request_code > 64000:
            self._next_request_code = self.REQUEST_CODE_BASE
        self._pending[code] = request
        try:
            self.activity.startActivityForResult(intent, code)
        except Exception as exc:
            self._pending.pop(code, None)
            self._report_error("Could not open Android folder access", exc)

    def _on_activity_result(self, request_code, result_code, intent):
        try:
            code = int(request_code)
        except Exception:
            return
        request = self._pending.pop(code, None)
        if request is None:
            return

        callback = request.get("callback")
        if int(result_code) != self.RESULT_OK or intent is None:
            if request["kind"] == "root" and callback:
                Clock.schedule_once(lambda _dt: callback(False), 0)
            return

        try:
            uri = intent.getData()
            if uri is None:
                raise RuntimeError("Android returned no folder URI.")
            if request["kind"] != "root":
                return

            grant_flags = int(intent.getFlags()) & (
                self.Intent.FLAG_GRANT_READ_URI_PERMISSION
                | self.Intent.FLAG_GRANT_WRITE_URI_PERMISSION
            )
            self.resolver.takePersistableUriPermission(uri, grant_flags)
            self.tree_uri = uri
            self.root_document_id = str(
                self.DocumentsContract.getTreeDocumentId(uri)
            )
            with open(self._root_state_path, "w", encoding="utf-8") as target:
                target.write(str(uri.toString()))
            if callback:
                Clock.schedule_once(lambda _dt: callback(True), 0)
        except Exception as exc:
            self._report_error("Could not save folder permission", exc)
            if callback:
                Clock.schedule_once(lambda _dt: callback(False), 0)

    @mainthread
    def _report_error(self, title, exc):
        message = str(exc).strip() or "Android storage operation failed."
        self.app_ref.set_status("File access failed", "#ef4444")
        self.app_ref.show_dialog(title, message[:1200])

    def _new_staging_dir(self, prefix):
        return tempfile.mkdtemp(prefix=prefix, dir=self.cache_root)

    @staticmethod
    def _safe_name(name):
        name = str(name or "").replace("\x00", "")
        name = os.path.basename(name)
        return "selected_file" if name in ("", ".", "..") else name

    def list_children(self, document_id):
        """Return direct child metadata for the in-app browser."""
        if self.tree_uri is None:
            raise RuntimeError("Choose a storage folder before browsing.")
        children_uri = self.DocumentsContract.buildChildDocumentsUriUsingTree(
            self.tree_uri, str(document_id)
        )
        columns = [
            self.Document.COLUMN_DOCUMENT_ID,
            self.Document.COLUMN_DISPLAY_NAME,
            self.Document.COLUMN_MIME_TYPE,
        ]
        cursor = self.resolver.query(children_uri, columns, None, None, None)
        if cursor is None:
            raise RuntimeError(
                "Android could not list this folder. Choose a different root folder."
            )

        items = []
        try:
            id_col = cursor.getColumnIndex(
                self.Document.COLUMN_DOCUMENT_ID
            )
            name_col = cursor.getColumnIndex(
                self.Document.COLUMN_DISPLAY_NAME
            )
            mime_col = cursor.getColumnIndex(
                self.Document.COLUMN_MIME_TYPE
            )
            if min(id_col, name_col, mime_col) < 0:
                raise RuntimeError(
                    "The folder provider returned incomplete file information."
                )
            while cursor.moveToNext():
                items.append((
                    str(cursor.getString(id_col)),
                    str(cursor.getString(name_col)),
                    str(cursor.getString(mime_col)),
                ))
        finally:
            cursor.close()
        return items

    def _document_uri(self, document_id):
        return self.DocumentsContract.buildDocumentUriUsingTree(
            self.tree_uri, str(document_id)
        )

    def copy_selected_file(self, document_id, display_name):
        """Copy one selected image to app-private storage."""
        name = self._safe_name(display_name)
        if not name.lower().endswith(self.IMAGE_EXTENSIONS):
            raise ValueError("Please select a PNG, JPG, JPEG, WEBP or BMP image.")
        local_dir = self._new_staging_dir("input_file_")
        local_path = os.path.join(local_dir, name)
        self._copy_uri_to_path(self._document_uri(document_id), local_path)
        return local_path

    def copy_input_folder(self, document_id):
        """Copy only radar*.png files required by Merge and TXD."""
        local_dir = self._new_staging_dir("input_")
        entries = self.list_children(document_id)
        wanted = [
            item for item in entries
            if item[2] != self.directory_mime
            and item[1].lower().startswith("radar")
            and item[1].lower().endswith(".png")
        ]
        wanted.sort(key=lambda item: item[1].lower())
        copied = 0
        for child_id, name, _mime in wanted:
            self._copy_uri_to_path(
                self._document_uri(child_id),
                os.path.join(local_dir, self._safe_name(name)),
            )
            copied += 1
        if copied:
            self.app_ref.set_status(
                f"Loaded {copied} radar PNG files", "#10b981"
            )
        else:
            self.app_ref.set_status(
                "No radar*.png files found in this folder", "#f59e0b"
            )
        return local_dir

    def create_output_folder(self, document_id):
        """Create a local staging folder mapped to a SAF destination folder."""
        local_dir = self._new_staging_dir("output_")
        self._output_dirs[os.path.realpath(local_dir)] = (
            self.tree_uri, str(document_id)
        )
        return local_dir

    def create_save_file(self, parent_document_id, display_name):
        """Create a destination document and return its local staging path."""
        name = self._safe_name(display_name)
        if not name.lower().endswith(".png"):
            name += ".png"
        existing = {
            item[1]: item[0]
            for item in self.list_children(parent_document_id)
        }
        self._delete_existing_child(
            parent_document_id, name, existing=existing
        )
        parent_uri = self._document_uri(parent_document_id)
        target_uri = self.DocumentsContract.createDocument(
            self.resolver, parent_uri, "image/png", name
        )
        if target_uri is None:
            raise RuntimeError(
                f"Android could not create {name!r} in the selected folder."
            )
        local_dir = self._new_staging_dir("save_")
        local_path = os.path.join(local_dir, name)
        self._output_files[os.path.realpath(local_path)] = (
            target_uri, name
        )
        return local_path

    def _copy_uri_to_path(self, uri, path):
        stream = self.resolver.openInputStream(uri)
        if stream is None:
            raise RuntimeError("Android could not open the selected file.")
        buffer = [0] * 65536
        try:
            with open(path, "wb") as output:
                while True:
                    count = int(stream.read(buffer))
                    if count < 0:
                        break
                    if count == 0:
                        continue
                    output.write(bytes(
                        (int(buffer[index]) & 0xff)
                        for index in range(count)
                    ))
        finally:
            stream.close()

    def _delete_existing_child(self, parent_document_id, name, existing=None):
        if existing is None:
            existing = {
                item[1]: item[0]
                for item in self.list_children(parent_document_id)
            }
        document_id = existing.get(name)
        if document_id is None:
            return
        document_uri = self._document_uri(document_id)
        deleted = self.DocumentsContract.deleteDocument(
            self.resolver, document_uri
        )
        if not deleted:
            raise RuntimeError(
                f"Cannot replace existing file {name!r} in the selected folder."
            )
        existing.pop(name, None)

    def _write_local_file_to_uri(self, local_path, uri):
        stream = self.resolver.openOutputStream(uri)
        if stream is None:
            raise RuntimeError("Android could not open the destination for writing.")
        try:
            with open(local_path, "rb") as source:
                while True:
                    chunk = source.read(65536)
                    if not chunk:
                        break
                    signed = [
                        value if value < 128 else value - 256
                        for value in chunk
                    ]
                    stream.write(signed)
            stream.flush()
        finally:
            stream.close()

    def _write_file_to_tree(self, parent_document_id, local_path, existing):
        filename = self._safe_name(os.path.basename(local_path))
        mime_type = mimetypes.guess_type(filename)[0] or (
            "application/octet-stream"
        )
        self._delete_existing_child(
            parent_document_id, filename, existing=existing
        )
        parent_uri = self._document_uri(parent_document_id)
        target_uri = self.DocumentsContract.createDocument(
            self.resolver, parent_uri, mime_type, filename
        )
        if target_uri is None:
            raise RuntimeError(
                f"Android could not create {filename!r} in the selected folder."
            )
        self._write_local_file_to_uri(local_path, target_uri)
        # Keep the in-memory child map in sync without querying the provider
        # again for each of the 144 output tiles.
        document_id = str(
            self.DocumentsContract.getDocumentId(target_uri)
        )
        existing[filename] = document_id

    def export_directory_if_needed(self, local_directory):
        mapping = self._output_dirs.get(os.path.realpath(local_directory))
        if mapping is None:
            return 0
        tree_uri, parent_document_id = mapping
        if not os.path.isdir(local_directory):
            raise RuntimeError("The app's staging output folder no longer exists.")

        files = sorted(
            os.path.join(local_directory, name)
            for name in os.listdir(local_directory)
            if os.path.isfile(os.path.join(local_directory, name))
        )
        if not files:
            return 0

        # One SAF directory query per export instead of one query per file.
        children = self.list_children(parent_document_id)
        existing = {name: doc_id for doc_id, name, _mime in children}
        for path in files:
            self._write_file_to_tree(parent_document_id, path, existing)
        return len(files)

    def export_file_if_needed(self, local_path):
        mapping = self._output_files.get(os.path.realpath(local_path))
        if mapping is None:
            return os.path.basename(local_path)
        target_uri, display_name = mapping
        self._write_local_file_to_uri(local_path, target_uri)
        self._output_files.pop(os.path.realpath(local_path), None)
        return display_name
