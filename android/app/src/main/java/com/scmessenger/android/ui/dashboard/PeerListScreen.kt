package com.scmessenger.android.ui.dashboard

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import com.scmessenger.android.R
import com.scmessenger.android.data.MeshRepository
import com.scmessenger.android.ui.components.ErrorBanner
import com.scmessenger.android.ui.components.IdenticonFromPeerId
import com.scmessenger.android.service.ConnectionQuality
import com.scmessenger.android.ui.components.ConnectionQualityIndicator
import com.scmessenger.android.ui.components.StatusIndicator
import com.scmessenger.android.ui.theme.*
import com.scmessenger.android.ui.viewmodels.DashboardViewModel
import com.scmessenger.android.utils.toEpochMillis
import java.text.SimpleDateFormat
import java.util.*

/**
 * Peer List screen - Display connected peers with transport info.
 *
 * Shows all active peers in the mesh network with:
 * - Identicon and peer ID
 * - Connection status and transport type
 * - Last seen timestamp
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PeerListScreen(
    onNavigateBack: () -> Unit,
    viewModel: DashboardViewModel = hiltViewModel()
) {
    val peers by viewModel.peers.collectAsState()
    val isLoading by viewModel.isLoading.collectAsState()
    val error by viewModel.error.collectAsState()

    LaunchedEffect(Unit) {
        viewModel.refreshData()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.peer_list_title)) },
                navigationIcon = {
                    IconButton(onClick = onNavigateBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.chat_action_dismiss))
                    }
                },
                actions = {
                    IconButton(onClick = { viewModel.refreshData() }) {
                        Icon(Icons.Default.Refresh, contentDescription = stringResource(R.string.diagnostics_action_refresh))
                    }
                }
            )
        }
    ) { paddingValues ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
        ) {
            when {
                isLoading -> {
                    CircularProgressIndicator(
                        modifier = Modifier.align(Alignment.Center)
                    )
                }

                peers.isEmpty() -> {
                    Column(
                        modifier = Modifier
                            .align(Alignment.Center)
                            .padding(32.dp),
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        Text(
                            text = stringResource(R.string.peer_list_no_peers),
                            style = MaterialTheme.typography.titleLarge
                        )

                        Spacer(modifier = Modifier.height(8.dp))

                        Text(
                            text = stringResource(R.string.peer_list_no_peers_description),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }

                else -> {
                    Column(modifier = Modifier.fillMaxSize()) {
                        // Error banner
                        error?.let {
                            ErrorBanner(
                                message = it,
                                onDismiss = { viewModel.clearError() }
                            )
                        }

                        // Peer count
                        Surface(
                            modifier = Modifier.fillMaxWidth(),
                            tonalElevation = 1.dp
                        ) {
                            val countText = if (peers.size == 1) {
                                stringResource(R.string.peer_list_count_format_singular, peers.size)
                            } else {
                                stringResource(R.string.peer_list_count_format_plural, peers.size)
                            }
                            Text(
                                text = countText,
                                modifier = Modifier.padding(16.dp),
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.Bold
                            )
                        }

                        // NODE-RELAY-LABEL-002: infrastructure relays get their own
                        // section, never mixed into the contacts/people list.
                        val regularPeers = peers.filterNot { it.isRelay }
                        val infrastructureRelays = peers.filter { it.isRelay }

                        // Peer list
                        LazyColumn(
                            modifier = Modifier.fillMaxSize(),
                            contentPadding = PaddingValues(16.dp),
                            verticalArrangement = Arrangement.spacedBy(12.dp)
                        ) {
                            item(key = "section-peers") {
                                Text(
                                    text = stringResource(R.string.peer_list_section_peers),
                                    style = MaterialTheme.typography.titleMedium,
                                    fontWeight = FontWeight.Bold
                                )
                            }
                            items(regularPeers, key = { it.peerId }) { peer ->
                                PeerCard(peer = peer)
                            }

                            if (infrastructureRelays.isNotEmpty()) {
                                item(key = "section-relays") {
                                    Column {
                                        Text(
                                            text = stringResource(R.string.peer_list_section_relays),
                                            style = MaterialTheme.typography.titleMedium,
                                            fontWeight = FontWeight.Bold,
                                            color = MaterialTheme.colorScheme.tertiary
                                        )
                                        Text(
                                            text = stringResource(R.string.shared_nodes_subtitle),
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant
                                        )
                                    }
                                }
                                items(infrastructureRelays, key = { "relay-${it.peerId}" }) { peer ->
                                    PeerCard(peer = peer, isInfrastructure = true)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PeerCard(
    peer: com.scmessenger.android.ui.viewmodels.PeerInfo,
    modifier: Modifier = Modifier,
    isInfrastructure: Boolean = false
) {
    Card(
        modifier = modifier.fillMaxWidth()
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            // Identicon
            IdenticonFromPeerId(
                peerId = peer.peerId,
                size = 56.dp
            )

            // Info
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                // Peer ID
                Text(
                    text = peer.displayName(),
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Medium
                )

                if (isInfrastructure) {
                    Text(
                        text = stringResource(R.string.peer_label_infrastructure_relay),
                        style = MaterialTheme.typography.labelSmall,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.tertiary
                    )
                }

                // NODE-TRANSPORT-VIS-001: one badge per known transport.
                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    peer.transports.ifEmpty { listOf(peer.transport) }.forEach { transport ->
                        TransportBadge(transport = transport)
                    }
                }

                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    StatusIndicator(
                        isOnline = peer.isOnline
                    )
                    ConnectionQualityIndicator(
                        quality = if (peer.isOnline) ConnectionQuality.GOOD
                                  else ConnectionQuality.UNKNOWN
                    )
                    Text(
                        text = if (peer.isOnline) stringResource(R.string.peer_list_status_online) else stringResource(R.string.peer_list_status_offline),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }

                // Last seen
                peer.lastSeen?.let {
                    Text(
                        text = stringResource(R.string.peer_list_last_seen, formatTimestamp(it)),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }
    }
}

private fun com.scmessenger.android.ui.viewmodels.PeerInfo.displayName(): String {
    val name = localNickname ?: nickname
    return if (!name.isNullOrEmpty()) name else peerId.take(16) + "..."
}

@Composable
private fun TransportBadge(
    transport: String,
    modifier: Modifier = Modifier
) {
    val color = when (transport) {
        "BLE" -> TransportBLE
        "WiFi Aware" -> TransportWiFiAware
        "WiFi Direct" -> TransportWiFiDirect
        "Internet" -> TransportInternet
        "TCP/LAN", "TCP/mDNS" -> TransportTcpLan
        MeshRepository.TRANSPORT_RELAY_CIRCUIT -> TransportRelayCircuit
        else -> MaterialTheme.colorScheme.surfaceVariant
    }

    Surface(
        modifier = modifier,
        color = color,
        shape = MaterialTheme.shapes.small
    ) {
        val unknownTransport = stringResource(R.string.unknown_transport)
        Text(
            text = transport,
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            style = MaterialTheme.typography.labelSmall,
            color = if (transport == unknownTransport) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onPrimary
        )
    }
}

private fun formatTimestamp(timestamp: ULong): String {
    val millis = timestamp.toEpochMillis()
    val date = Date(millis)
    val now = Date()

    val diff = (now.time - date.time) / 1000

    return when {
        diff < 60 -> "just now"
        diff < 3600 -> "${diff / 60}m ago"
        diff < 86400 -> "${diff / 3600}h ago"
        else -> {
            val sdf = SimpleDateFormat("MMM d", Locale.getDefault())
            sdf.format(date)
        }
    }
}
