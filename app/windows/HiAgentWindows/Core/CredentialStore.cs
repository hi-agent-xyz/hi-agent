using System.Runtime.InteropServices;
using System.Text;

namespace HiAgent.Windows.Core;

/// <summary>
/// Long-lived credentials, in Windows Credential Manager.
///
/// `docs/arch/topology.md`: "credentials live in the OS keychain, never a plist
/// or localStorage". Credential Manager is what the sentence means on Windows —
/// the store the Keychain and the Android Keystore stand in for elsewhere. A
/// DPAPI-encrypted file would protect the bytes about as well and would still
/// be a secret this app invented a home for; the OS has one.
///
/// Note what is *not* stored: the session cookie. That belongs to WebView2's
/// profile and is short-lived by design. Only the credential the face never
/// sees is here.
/// </summary>
internal static class CredentialStore
{
    private const uint CredTypeGeneric = 1;

    /// <summary>
    /// Persist across sign-outs on this machine, not roamed to others. A
    /// credential names a device at the core ("Xiaoyuan's PC"), so carrying it
    /// to a second machine would make two devices share one entry — and the
    /// device list at the core is what makes revocation mean anything.
    /// </summary>
    private const uint CredPersistLocalMachine = 2;

    private static string TargetName(string coreId) => $"HiAgent:core:{coreId}";

    internal static void Save(string coreId, string credential)
    {
        var blob = Encoding.UTF8.GetBytes(credential);
        var target = Marshal.StringToCoTaskMemUni(TargetName(coreId));
        var user = Marshal.StringToCoTaskMemUni("hi-agent");
        var data = Marshal.AllocCoTaskMem(blob.Length);
        try
        {
            Marshal.Copy(blob, 0, data, blob.Length);
            var credentialStruct = new Credential
            {
                Type = CredTypeGeneric,
                TargetName = target,
                CredentialBlobSize = (uint)blob.Length,
                CredentialBlob = data,
                Persist = CredPersistLocalMachine,
                UserName = user,
            };
            if (!CredWriteW(ref credentialStruct, 0))
            {
                throw new InvalidOperationException(
                    $"CredWrite failed ({Marshal.GetLastWin32Error()})");
            }
        }
        finally
        {
            Marshal.ZeroFreeCoTaskMemUnicode(target);
            Marshal.ZeroFreeCoTaskMemUnicode(user);
            // The credential's own bytes are wiped before the memory goes back.
            for (var i = 0; i < blob.Length; i++)
            {
                Marshal.WriteByte(data, i, 0);
            }
            Marshal.FreeCoTaskMem(data);
            Array.Clear(blob);
        }
    }

    internal static string? Load(string coreId)
    {
        if (!CredReadW(TargetName(coreId), CredTypeGeneric, 0, out var handle))
        {
            return null;
        }
        try
        {
            var credential = Marshal.PtrToStructure<Credential>(handle);
            if (credential.CredentialBlob == IntPtr.Zero || credential.CredentialBlobSize == 0)
            {
                return null;
            }
            var bytes = new byte[credential.CredentialBlobSize];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            return Encoding.UTF8.GetString(bytes);
        }
        finally
        {
            CredFree(handle);
        }
    }

    /// <summary>
    /// Forget a core. Idempotent: a credential that is already gone is the state
    /// this asks for, so a failed delete is not an error worth raising.
    /// </summary>
    internal static void Delete(string coreId)
    {
        if (!CredDeleteW(TargetName(coreId), CredTypeGeneric, 0))
        {
            Log.Write($"CredDelete for {coreId}: {Marshal.GetLastWin32Error()}");
        }
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct Credential
    {
        public uint Flags;
        public uint Type;
        public IntPtr TargetName;
        public IntPtr Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public IntPtr Attributes;
        public IntPtr TargetAlias;
        public IntPtr UserName;
    }

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CredWriteW([In] ref Credential credential, uint flags);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CredReadW(string target, uint type, uint flags, out IntPtr credential);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CredDeleteW(string target, uint type, uint flags);

    [DllImport("advapi32.dll")]
    private static extern void CredFree(IntPtr buffer);
}
