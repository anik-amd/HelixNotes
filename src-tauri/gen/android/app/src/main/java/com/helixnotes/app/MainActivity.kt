package com.helixnotes.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.WebView
import android.webkit.MimeTypeMap
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import java.io.File

class MainActivity : TauriActivity() {
  private var webView: WebView? = null

  companion object {
    private const val STORAGE_PERMISSION_REQUEST_CODE = 1001
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    this.webView = webView
    webView.addJavascriptInterface(StorageBridge(this), "Android")
    // Render text at the size CSS specifies, ignoring the device's system font-size
    // accessibility setting (textZoom is otherwise tied to the system font scale). (#100)
    webView.settings.textZoom = 100
    // The actual fix: disable WebView "text autosizing" / font-boosting, which recomputes
    // paragraph font-size and line-height from viewport heuristics and ignores our CSS (only
    // size + line-height were affected; font-family was fine). NORMAL turns it off. (#100)
    webView.settings.layoutAlgorithm = android.webkit.WebSettings.LayoutAlgorithm.NORMAL
  }

  override fun onPause() {
    // Give the editor a chance to persist its current document before Android
    // suspends the WebView. The save remains best-effort when the process is killed.
    webView?.evaluateJavascript(
      "window.dispatchEvent(new Event('app-lifecycle-flush'))",
      null
    )
    super.onPause()
  }

  override fun onResume() {
    super.onResume()
    if (hasStoragePermission()) {
      webView?.evaluateJavascript(
        "window.__storagePermissionGranted && window.__storagePermissionGranted()",
        null
      )
    }
  }

  fun hasStoragePermission(): Boolean {
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      // Android 11+ (API 30+): "All files access" special permission.
      Environment.isExternalStorageManager()
    } else {
      // Android 7-10 (API 24-29): legacy runtime storage permission.
      ContextCompat.checkSelfPermission(this, Manifest.permission.WRITE_EXTERNAL_STORAGE) ==
        PackageManager.PERMISSION_GRANTED
    }
  }

  fun requestStoragePermission() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      if (Environment.isExternalStorageManager()) return
      try {
        val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
        intent.data = Uri.parse("package:$packageName")
        startActivity(intent)
      } catch (e: Exception) {
        try {
          val fallback = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
          startActivity(fallback)
        } catch (_: Exception) {
          val details = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
          details.data = Uri.parse("package:$packageName")
          startActivity(details)
        }
      }
    } else {
      // Android 7-10: request the legacy runtime permission directly.
      ActivityCompat.requestPermissions(
        this,
        arrayOf(
          Manifest.permission.READ_EXTERNAL_STORAGE,
          Manifest.permission.WRITE_EXTERNAL_STORAGE
        ),
        STORAGE_PERMISSION_REQUEST_CODE
      )
    }
  }

  override fun onRequestPermissionsResult(
    requestCode: Int,
    permissions: Array<out String>,
    grantResults: IntArray
  ) {
    super.onRequestPermissionsResult(requestCode, permissions, grantResults)
    if (requestCode == STORAGE_PERMISSION_REQUEST_CODE && hasStoragePermission()) {
      webView?.evaluateJavascript(
        "window.__storagePermissionGranted && window.__storagePermissionGranted()",
        null
      )
    }
  }

  fun prepareVaultDir(path: String): Boolean {
    return try {
      val dir = File(path)
      dir.mkdirs()
      File(dir, ".helixnotes/trash").mkdirs()
      File(dir, ".helixnotes/attachments").mkdirs()
      dir.exists() && dir.canWrite()
    } catch (e: Exception) {
      false
    }
  }

  fun openFile(path: String) {
    try {
      val file = File(path)
      if (!file.exists()) return
      val uri = FileProvider.getUriForFile(this, "${packageName}.fileprovider", file)
      val ext = MimeTypeMap.getFileExtensionFromUrl(path)
      val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "*/*"
      val intent = Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(uri, mime)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      startActivity(intent)
    } catch (e: Exception) {
      android.util.Log.e("HelixNotes", "Failed to open file: $path", e)
    }
  }
}

class StorageBridge(private val activity: MainActivity) {
  @JavascriptInterface
  fun hasStoragePermission(): Boolean {
    return activity.hasStoragePermission()
  }

  @JavascriptInterface
  fun requestStoragePermission() {
    activity.runOnUiThread {
      activity.requestStoragePermission()
    }
  }

  @JavascriptInterface
  fun prepareVaultDir(path: String): Boolean {
    return activity.prepareVaultDir(path)
  }

  @JavascriptInterface
  fun openFile(path: String) {
    activity.runOnUiThread {
      activity.openFile(path)
    }
  }
}
