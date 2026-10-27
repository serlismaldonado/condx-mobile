package com.example.condxmobile;

import android.app.Activity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Elegir un archivo para adjuntar a un hilo. La actividad de nsh no recibe resultados de otras apps,
 * así que esta (sin pantalla propia) abre el selector del sistema, copia lo elegido a
 * files/picked/file.bin, escribe files/picked/meta.txt (nombre y tipo, al final: es la señal de que
 * terminó) y se cierra. Condx lo lee desde Rust (scan.rs). Si se cancela, escribe cancel.txt.
 */
public class PickFile extends Activity {
    private static final long MAX = 9L * 1024 * 1024;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        if (state != null) return;
        Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        i.addCategory(Intent.CATEGORY_OPENABLE);
        i.setType("*/*");
        startActivityForResult(i, 1);
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        File dir = new File(getFilesDir(), "picked");
        dir.mkdirs();
        try {
            if (result != RESULT_OK || data == null || data.getData() == null) {
                write(new File(dir, "cancel.txt"), "cancel");
            } else {
                Uri uri = data.getData();
                String name = "file";
                Cursor c = getContentResolver().query(uri, null, null, null, null);
                if (c != null) {
                    int ix = c.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    if (ix >= 0 && c.moveToFirst() && c.getString(ix) != null) name = c.getString(ix);
                    c.close();
                }
                String mime = getContentResolver().getType(uri);
                if (mime == null) mime = "application/octet-stream";
                long total = 0;
                try (InputStream in = getContentResolver().openInputStream(uri); OutputStream out = new FileOutputStream(new File(dir, "file.bin"))) {
                    byte[] buf = new byte[64 * 1024];
                    int n;
                    while ((n = in.read(buf)) > 0) {
                        total += n;
                        if (total > MAX) throw new Exception("the file is bigger than 9 MB");
                        out.write(buf, 0, n);
                    }
                }
                write(new File(dir, "meta.txt"), mime + "\n" + name.replace('\n', ' '));
            }
        } catch (Exception e) {
            try {
                write(new File(dir, "error.txt"), String.valueOf(e.getMessage()));
            } catch (Exception ignored) {
            }
        }
        finish();
    }

    private static void write(File f, String text) throws Exception {
        File tmp = new File(f.getPath() + ".tmp");
        try (OutputStream out = new FileOutputStream(tmp)) {
            out.write(text.getBytes("UTF-8"));
        }
        tmp.renameTo(f);
    }
}
