package radio.ks3ckc.ft8af.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import com.k1af.ft8af.MainViewModel
import com.k1af.ft8af.R
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import radio.ks3ckc.ft8af.lotw.LotwClient
import radio.ks3ckc.ft8af.theme.*

/**
 * LoTW username/password and the confirmations download (fork). Credentials live in
 * the config table (lotwUsername/lotwPassword, both "sensitive" for the settings backup).
 */
@Composable
internal fun LotwDialog(
    mainViewModel: MainViewModel,
    initialUsername: String,
    initialPassword: String,
    onDismiss: () -> Unit,
    onSaved: (username: String) -> Unit,
) {
    var user by remember { mutableStateOf(initialUsername) }
    var pass by remember { mutableStateOf(initialPassword) }
    var busy by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf<String?>(null) }
    var statusOk by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val db = mainViewModel.databaseOpr

    fun save() {
        db.writeConfig(LotwClient.CONFIG_USERNAME, user.trim(), null)
        db.writeConfig(LotwClient.CONFIG_PASSWORD, pass, null)
        onSaved(user.trim())
    }

    fun download(full: Boolean) {
        save()
        busy = true
        status = null
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                val last = db.getAllConfigSync()[LotwClient.CONFIG_LAST_QSL]
                LotwClient.downloadConfirmations(
                    db.db, user, pass, last, full,
                    saveLastQsl = { db.writeConfig(LotwClient.CONFIG_LAST_QSL, it, null) },
                )
            }
            busy = false
            statusOk = result.error == null
            status = result.error
                ?: "${result.newlyConfirmed} QSOs newly confirmed (${result.inReport} in LoTW report)"
        }
    }

    val fieldColors = OutlinedTextFieldDefaults.colors(
        focusedTextColor = TextPrimary,
        unfocusedTextColor = TextPrimary,
        cursorColor = Accent,
        focusedBorderColor = Accent,
        unfocusedBorderColor = BorderStrong,
        focusedLabelColor = Accent,
        unfocusedLabelColor = TextMuted,
    )

    Dialog(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(16.dp))
                .background(BgSurface2)
                .padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text("LoTW", color = TextPrimary, fontWeight = FontWeight.SemiBold, fontSize = 18.sp)
            Text(
                text = "Your lotw.arrl.org website login. Downloads your LoTW confirmations and " +
                    "marks matching QSOs confirmed. Uploading to LoTW still needs TQSL " +
                    "(it signs each QSO with your certificate).",
                color = TextMuted,
                fontSize = 12.sp,
                lineHeight = 16.sp,
            )
            OutlinedTextField(
                value = user,
                onValueChange = { user = it; status = null },
                label = { Text(stringResource(R.string.settings_username)) },
                singleLine = true,
                colors = fieldColors,
                textStyle = TextStyle(fontSize = 14.sp),
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = pass,
                onValueChange = { pass = it; status = null },
                label = { Text(stringResource(R.string.settings_password)) },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                colors = fieldColors,
                textStyle = TextStyle(fontSize = 14.sp),
                modifier = Modifier.fillMaxWidth(),
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                TextButton(onClick = { download(full = false) }, enabled = !busy) {
                    Text(
                        "Download new",
                        color = if (busy) TextFaint else Accent,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
                TextButton(onClick = { download(full = true) }, enabled = !busy) {
                    Text("Download all", color = if (busy) TextFaint else Accent)
                }
                if (busy) {
                    CircularProgressIndicator(
                        modifier = Modifier.width(16.dp).height(16.dp),
                        strokeWidth = 2.dp,
                        color = Accent,
                    )
                }
            }
            status?.let {
                Text(
                    it,
                    color = if (statusOk) StatusConfirmed else StatusBad,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
            ) {
                TextButton(onClick = onDismiss) {
                    Text(stringResource(R.string.action_cancel), color = TextMuted)
                }
                TextButton(onClick = { save(); onDismiss() }) {
                    Text(stringResource(R.string.action_save), color = Accent, fontWeight = FontWeight.SemiBold)
                }
            }
        }
    }
}
