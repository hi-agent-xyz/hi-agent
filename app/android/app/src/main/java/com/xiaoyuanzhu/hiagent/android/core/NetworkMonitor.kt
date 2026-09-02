package com.xiaoyuanzhu.hiagent.android.core

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.distinctUntilChanged

/**
 * Whether this handset has a network at all — the `NWPathMonitor` opposite
 * number, and the thing that tells an offline screen apart from an unreachable
 * core.
 *
 * Deliberately not `NET_CAPABILITY_VALIDATED`: a core on the LAN is reachable
 * from a Wi-Fi network with no route to the internet, and treating that as
 * offline would refuse to open the one core that was actually available.
 */
class NetworkMonitor(context: Context) {
    private val connectivity = context.applicationContext
        .getSystemService(ConnectivityManager::class.java)

    val isConnected: Flow<Boolean> = callbackFlow {
        val callback = object : ConnectivityManager.NetworkCallback() {
            private val live = mutableSetOf<Network>()

            override fun onAvailable(network: Network) {
                live += network
                trySend(true)
            }

            override fun onLost(network: Network) {
                live -= network
                trySend(live.isNotEmpty())
            }
        }

        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()

        trySend(hasNetworkNow())
        connectivity?.registerNetworkCallback(request, callback)
        awaitClose { runCatching { connectivity?.unregisterNetworkCallback(callback) } }
    }.distinctUntilChanged()

    private fun hasNetworkNow(): Boolean {
        val active = connectivity?.activeNetwork ?: return false
        val caps = connectivity.getNetworkCapabilities(active) ?: return false
        return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
    }
}
