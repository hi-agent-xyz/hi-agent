package com.xiaoyuanzhu.hiagent.android.core

import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import org.junit.Test

/**
 * The cleartext rule is the one piece of policy this client states itself rather
 * than inheriting from the platform — Android's network security config cannot
 * express "private address ranges", so `normalizeBaseUrl` carries what
 * `NSAllowsLocalNetworking` gives iOS for free. That makes it worth pinning.
 */
class CoreClientTest {
    @Test
    fun `https is accepted for any host`() {
        assertEquals(
            "https://hi-agent.xyz/ana",
            CoreClient.normalizeBaseUrl("https://hi-agent.xyz/ana").toString(),
        )
    }

    @Test
    fun `http is accepted on the local network`() {
        listOf(
            "http://192.168.1.24:12358",
            "http://10.0.0.4:12358",
            "http://172.16.2.10:12358",
            "http://127.0.0.1:12358",
            "http://169.254.10.2:12358",
            "http://localhost:12358",
            "http://core.local:12358",
            "http://raspberrypi:12358",
        ).forEach { address ->
            CoreClient.normalizeBaseUrl(address)
        }
    }

    @Test
    fun `http is refused for a public host`() {
        listOf(
            "http://hi-agent.xyz",
            "http://example.com/ana",
            "http://8.8.8.8:12358",
            "http://172.32.0.1:12358", // just outside 172.16/12
        ).forEach { address ->
            assertFailsWith<CoreClientException.InvalidAddress>(address) {
                CoreClient.normalizeBaseUrl(address)
            }
        }
    }

    @Test
    fun `credentials in the address are refused`() {
        assertFailsWith<CoreClientException.InvalidAddress> {
            CoreClient.normalizeBaseUrl("https://someone:secret@hi-agent.xyz")
        }
    }

    @Test
    fun `a non-http scheme is refused`() {
        assertFailsWith<CoreClientException.InvalidAddress> {
            CoreClient.normalizeBaseUrl("ftp://hi-agent.xyz")
        }
        assertFailsWith<CoreClientException.InvalidAddress> {
            CoreClient.normalizeBaseUrl("not a url at all")
        }
    }

    /**
     * Two spellings of one core must land on one roster entry, or switching
     * between them re-pairs instead of reconnecting.
     */
    @Test
    fun `query, fragment and trailing slash are normalised away`() {
        val canonical = CoreClient.normalizeBaseUrl("https://hi-agent.xyz/ana").toString()
        listOf(
            "https://hi-agent.xyz/ana/",
            "https://hi-agent.xyz/ana?x=1",
            "https://hi-agent.xyz/ana#top",
            "  https://hi-agent.xyz/ana  ",
        ).forEach { variant ->
            assertEquals(canonical, CoreClient.normalizeBaseUrl(variant).toString(), variant)
        }
    }

    /** A core at a subpath keeps it: its session endpoint is `/ana/api/session`. */
    @Test
    fun `endpoints keep the core's subpath`() {
        val base = CoreClient.normalizeBaseUrl("https://hi-agent.xyz/ana")
        assertEquals(
            "https://hi-agent.xyz/ana/api/session",
            CoreClient.endpoint(base, "api/session").toString(),
        )

        val root = CoreClient.normalizeBaseUrl("http://192.168.1.24:12358")
        assertEquals(
            "http://192.168.1.24:12358/healthz",
            CoreClient.endpoint(root, "healthz").toString(),
        )
    }

    @Test
    fun `local host detection covers ipv6 loopback and unique-local`() {
        assertTrue(CoreClient.isLocalHost("::1"))
        assertTrue(CoreClient.isLocalHost("[::1]"))
        assertTrue(CoreClient.isLocalHost("fd00::1"))
        assertTrue(CoreClient.isLocalHost("fe80::1"))
        assertFalse(CoreClient.isLocalHost("2001:4860:4860::8888"))
    }
}
