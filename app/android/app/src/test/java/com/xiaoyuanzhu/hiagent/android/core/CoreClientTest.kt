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
    /**
     * The trailing slash is `HttpUrl`'s, not ours: a URL with an empty path does not
     * exist in OkHttp's model, so the root renders as `/` however it was typed. The
     * iOS client's `URLComponents` keeps the path genuinely empty and its canonical
     * form has no slash — the two strings differ and that is fine, because each
     * device's roster is its own and every path is built by
     * [CoreClient.endpoint], which trims before joining.
     *
     * This assertion was written to the iOS spelling and had never passed.
     */
    @Test
    fun `https is accepted for any host`() {
        assertEquals(
            "https://ana.hi-agent.xyz/",
            CoreClient.normalizeBaseUrl("https://ana.hi-agent.xyz").toString(),
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
        val canonical = CoreClient.normalizeBaseUrl("https://ana.hi-agent.xyz").toString()
        listOf(
            "https://ana.hi-agent.xyz/",
            "https://ana.hi-agent.xyz?x=1",
            "https://ana.hi-agent.xyz#top",
            "  https://ana.hi-agent.xyz  ",
        ).forEach { variant ->
            assertEquals(canonical, CoreClient.normalizeBaseUrl(variant).toString(), variant)
        }
    }

    /**
     * A relayed core is at the root of its own origin, but a self-hosted one may
     * sit behind somebody's own reverse proxy at a path — so a base that carries
     * one keeps it.
     */
    @Test
    fun `endpoints keep a path the base carries`() {
        val relayed = CoreClient.normalizeBaseUrl("https://ana.hi-agent.xyz")
        assertEquals(
            "https://ana.hi-agent.xyz/api/session",
            CoreClient.endpoint(relayed, "api/session").toString(),
        )

        val behindAProxy = CoreClient.normalizeBaseUrl("https://example.com/agent")
        assertEquals(
            "https://example.com/agent/api/session",
            CoreClient.endpoint(behindAProxy, "api/session").toString(),
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
