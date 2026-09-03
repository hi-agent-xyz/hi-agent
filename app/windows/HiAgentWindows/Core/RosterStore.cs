using System.Text.Json;

namespace HiAgent.Windows.Core;

/// <summary>
/// The roster on disk. Addresses and labels only — the credential for an entry
/// lives in <see cref="CredentialStore"/>, keyed by the same id.
///
/// Rosters do not sync between apps, which falls out of the design rather than
/// being a rule: adding a core means acquiring a credential for it, and a
/// credential is issued to one surface.
/// </summary>
internal sealed class RosterStore
{
    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        PropertyNameCaseInsensitive = true,
    };

    private readonly object _gate = new();
    private RosterSnapshot _snapshot = new();

    internal IReadOnlyList<RosterEntry> Entries
    {
        get
        {
            lock (_gate)
            {
                return _snapshot.Entries.ToList();
            }
        }
    }

    internal string? AttachedId
    {
        get
        {
            lock (_gate)
            {
                return _snapshot.AttachedId;
            }
        }
    }

    internal void Load()
    {
        lock (_gate)
        {
            try
            {
                if (File.Exists(AppPaths.RosterFile))
                {
                    var text = File.ReadAllText(AppPaths.RosterFile);
                    _snapshot = JsonSerializer.Deserialize<RosterSnapshot>(text, Json) ?? new RosterSnapshot();
                }
            }
            catch (Exception e)
            {
                // A roster that will not parse is recoverable — the entries can
                // be added again, and the alternative is an app that will not
                // start. The file is left where it is so it can be looked at.
                Log.Write($"roster unreadable, starting empty: {e.Message}");
                _snapshot = new RosterSnapshot();
            }
        }
    }

    internal RosterEntry? Find(string id)
    {
        lock (_gate)
        {
            return _snapshot.Entries.FirstOrDefault(e => e.Id == id);
        }
    }

    internal RosterEntry? Local()
    {
        lock (_gate)
        {
            return _snapshot.Entries.FirstOrDefault(e => e.IsLocal);
        }
    }

    internal RosterEntry? Attached()
    {
        lock (_gate)
        {
            return _snapshot.AttachedId is { } id
                ? _snapshot.Entries.FirstOrDefault(e => e.Id == id)
                : null;
        }
    }

    /// <summary>Add or update an entry, then persist.</summary>
    internal void Put(RosterEntry entry)
    {
        lock (_gate)
        {
            var existing = _snapshot.Entries.FirstOrDefault(e => e.Id == entry.Id);
            if (existing is null)
            {
                _snapshot.Entries.Add(entry);
            }
            else
            {
                existing.BaseUrl = entry.BaseUrl;
                existing.Label = entry.Label;
            }
            Save();
        }
    }

    internal void Attach(string? id)
    {
        lock (_gate)
        {
            _snapshot.AttachedId = id;
            Save();
        }
    }

    /// <summary>
    /// Forget a core: the entry and its credential together. The local engine
    /// cannot be forgotten — it is this machine, and removing it would leave the
    /// shell supervising a process it has no entry for.
    /// </summary>
    internal void Remove(string id)
    {
        lock (_gate)
        {
            var entry = _snapshot.Entries.FirstOrDefault(e => e.Id == id);
            if (entry is null || entry.IsLocal)
            {
                return;
            }
            _snapshot.Entries.Remove(entry);
            if (_snapshot.AttachedId == id)
            {
                _snapshot.AttachedId = _snapshot.Entries.FirstOrDefault()?.Id;
            }
            Save();
        }
        CredentialStore.Delete(id);
    }

    /// <summary>Caller holds <see cref="_gate"/>.</summary>
    private void Save()
    {
        try
        {
            var text = JsonSerializer.Serialize(_snapshot, Json);
            // Write beside and move, so an interrupted save cannot leave a
            // half-written roster where a whole one was.
            var temp = AppPaths.RosterFile + ".tmp";
            File.WriteAllText(temp, text);
            File.Move(temp, AppPaths.RosterFile, overwrite: true);
        }
        catch (Exception e)
        {
            Log.Write($"roster not saved: {e.Message}");
        }
    }
}
