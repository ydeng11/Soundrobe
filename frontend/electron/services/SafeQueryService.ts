/**
 * SafeQueryService — typed read-only metadata and database queries.
 *
 * No raw SQL exposed in v1. Every query is a typed method.
 */

import type { TrackData } from "../handlers/tracks";
import type { LibrarySummary } from "./LibraryService";

export interface FindTracksQuery {
  title?: string;
  artist?: string;
  album?: string;
  genre?: string;
  year?: string;
  codec?: string;
  missingTitle?: boolean;
  missingArtist?: boolean;
  missingAlbum?: boolean;
  missingYear?: boolean;
  missingGenre?: boolean;
  missingCover?: boolean;
  hasDuplicates?: boolean;
}

export interface AggregateSummary {
  totalTracks: number;
  totalAlbums: number;
  totalArtists: number;
  totalGenres: number;
  byAlbum: Record<string, number>;
  byArtist: Record<string, number>;
  byGenre: Record<string, number>;
  byYear: Record<string, number>;
  byCodec: Record<string, number>;
  tagCompleteness: {
    title: number;
    artist: number;
    album: number;
    year: number;
    genre: number;
  };
}

export class SafeQueryService {
  private tracks: TrackData[] = [];

  /**
   * Set the current track list for querying.
   */
  setTracks(tracks: TrackData[]): void {
    this.tracks = tracks;
  }

  /**
   * Find tracks matching specific criteria.
   */
  findTracks(query: FindTracksQuery): TrackData[] {
    let results = [...this.tracks];

    if (query.title) {
      const lower = query.title.toLowerCase();
      results = results.filter(
        (t) => t.title?.toLowerCase().includes(lower),
      );
    }

    if (query.artist) {
      const lower = query.artist.toLowerCase();
      results = results.filter(
        (t) =>
          t.artist?.toLowerCase().includes(lower) ||
          t.albumArtist?.toLowerCase().includes(lower),
      );
    }

    if (query.album) {
      const lower = query.album.toLowerCase();
      results = results.filter(
        (t) => t.album?.toLowerCase().includes(lower),
      );
    }

    if (query.genre) {
      const lower = query.genre.toLowerCase();
      results = results.filter(
        (t) => t.genre?.toLowerCase().includes(lower),
      );
    }

    if (query.year) {
      results = results.filter((t) => t.year === query.year);
    }

    if (query.codec) {
      const lower = query.codec.toLowerCase();
      results = results.filter(
        (t) => t.codec.toLowerCase() === lower,
      );
    }

    if (query.missingTitle) {
      results = results.filter(
        (t) => !t.title || t.title.trim() === "",
      );
    }

    if (query.missingArtist) {
      results = results.filter(
        (t) => !t.artist || t.artist.trim() === "",
      );
    }

    if (query.missingAlbum) {
      results = results.filter(
        (t) => !t.album || t.album.trim() === "",
      );
    }

    if (query.missingYear) {
      results = results.filter(
        (t) => !t.year || t.year.trim() === "",
      );
    }

    if (query.missingGenre) {
      results = results.filter(
        (t) => !t.genre || t.genre.trim() === "",
      );
    }

    if (query.missingCover) {
      results = results.filter((t) => !t.hasCover);
    }

    if (query.hasDuplicates) {
      const seen = new Map<string, TrackData[]>();
      for (const track of results) {
        const title = this.nonBlank(track.title);
        const artist = this.nonBlank(track.artist);
        const album = this.nonBlank(track.album);
        if (!title || !artist || !album) continue;

        const key = `${title}|${artist}|${album}`.toLowerCase();
        if (!seen.has(key)) seen.set(key, []);
        seen.get(key)!.push(track);
      }
      results = [];
      for (const [, group] of seen) {
        if (group.length > 1) results.push(...group);
      }
    }

    return results;
  }

  /**
   * Compute aggregate counts from the current track list.
   */
  aggregate(): AggregateSummary {
    const albums = new Set<string>();
    const artists = new Set<string>();
    const genres = new Set<string>();
    const years = new Set<string>();
    const codecs: Record<string, number> = {};
    const byAlbum: Record<string, number> = {};
    const byArtist: Record<string, number> = {};
    const byGenre: Record<string, number> = {};
    const byYear: Record<string, number> = {};

    let hasTitle = 0;
    let hasArtist = 0;
    let hasAlbum = 0;
    let hasYear = 0;
    let hasGenre = 0;

    for (const track of this.tracks) {
      const album = this.nonBlank(track.album);
      const artist = this.nonBlank(track.artist);
      const albumArtist = this.nonBlank(track.albumArtist);
      const genre = this.nonBlank(track.genre);
      const year = this.nonBlank(track.year);

      if (album) {
        albums.add(album);
        byAlbum[album] = (byAlbum[album] ?? 0) + 1;
      }
      if (artist) {
        artists.add(artist);
        byArtist[artist] = (byArtist[artist] ?? 0) + 1;
      }
      if (albumArtist) artists.add(albumArtist);
      if (genre) {
        genres.add(genre);
        byGenre[genre] = (byGenre[genre] ?? 0) + 1;
      }
      if (year) {
        years.add(year);
        byYear[year] = (byYear[year] ?? 0) + 1;
      }
      codecs[track.codec] = (codecs[track.codec] ?? 0) + 1;

      if (track.title && track.title.trim()) hasTitle++;
      if (track.artist && track.artist.trim()) hasArtist++;
      if (track.album && track.album.trim()) hasAlbum++;
      if (track.year && track.year.trim()) hasYear++;
      if (track.genre && track.genre.trim()) hasGenre++;
    }

    const total = this.tracks.length || 1;

    return {
      totalTracks: this.tracks.length,
      totalAlbums: albums.size,
      totalArtists: artists.size,
      totalGenres: genres.size,
      byAlbum,
      byArtist,
      byGenre,
      byYear,
      byCodec: codecs,
      tagCompleteness: {
        title: Math.round((hasTitle / total) * 100),
        artist: Math.round((hasArtist / total) * 100),
        album: Math.round((hasAlbum / total) * 100),
        year: Math.round((hasYear / total) * 100),
        genre: Math.round((hasGenre / total) * 100),
      },
    };
  }

  private nonBlank(value: string | null | undefined): string | null {
    if (!value) return null;
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : null;
  }
}
