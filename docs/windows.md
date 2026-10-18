# So you're on Windows

<p align="center"><i>Please remain calm. This is a safe space.</i></p>

We have some news, and we'll be gentle about it: **CrustDB runs on Linux x86_64 only.**
No `.exe`, no `.msi`, no "Install now and restart 4 times".

We looked into a Windows port. Then the update dialog showed up and asked us to keep the
computer on, and we haven't seen it since.

## What you'll be leaving behind

- That restart for an update you didn't ask for, right as you hit "Run query"
- The Start menu showing you the news when you searched for `notepad`
- `C:\Program Files (x86)\` and its quiet sense of mystery
- Reinstalling the whole OS once a year "to clean things up"

## What you get instead

- A terminal you will be weirdly proud of after a week
- Updates that wait until **you** say so
- CrustDB, running natively, on X11 or Wayland
- Bragging rights in every meeting. Use them wisely.

## Pick your new home

Any of the distros below runs CrustDB. Not sure which one? Pick Ubuntu or Linux Mint and come back
later to argue about it on the internet like everyone else.

<table>
  <tr>
    <td align="center" width="25%">
      <a href="https://ubuntu.com/download/desktop"><img src="https://cdn.simpleicons.org/ubuntu" width="64" alt="Ubuntu"><br><b>Ubuntu</b></a><br>
      <sub>The "it just works" one</sub>
    </td>
    <td align="center" width="25%">
      <a href="https://linuxmint.com/download.php"><img src="https://cdn.simpleicons.org/linuxmint" width="64" alt="Linux Mint"><br><b>Linux Mint</b></a><br>
      <sub>Feels like home, minus the ads</sub>
    </td>
    <td align="center" width="25%">
      <a href="https://fedoraproject.org/workstation/download"><img src="https://cdn.simpleicons.org/fedora" width="64" alt="Fedora"><br><b>Fedora</b></a><br>
      <sub>Lives in the future, on purpose</sub>
    </td>
    <td align="center" width="25%">
      <a href="https://www.debian.org/distrib/"><img src="https://cdn.simpleicons.org/debian" width="64" alt="Debian"><br><b>Debian</b></a><br>
      <sub>Older than some of your coworkers</sub>
    </td>
  </tr>
  <tr>
    <td align="center" width="25%">
      <a href="https://pop.system76.com/"><img src="https://cdn.simpleicons.org/popos" width="64" alt="Pop!_OS"><br><b>Pop!_OS</b></a><br>
      <sub>For devs who like tiling</sub>
    </td>
    <td align="center" width="25%">
      <a href="https://manjaro.org/products/download/x86"><img src="https://cdn.simpleicons.org/manjaro" width="64" alt="Manjaro"><br><b>Manjaro</b></a><br>
      <sub>Arch, but with training wheels</sub>
    </td>
    <td align="center" width="25%">
      <a href="https://get.opensuse.org/"><img src="https://cdn.simpleicons.org/opensuse" width="64" alt="openSUSE"><br><b>openSUSE</b></a><br>
      <sub>Has a chameleon. Enough said.</sub>
    </td>
    <td align="center" width="25%">
      <a href="https://archlinux.org/download/"><img src="https://cdn.simpleicons.org/archlinux" width="64" alt="Arch Linux"><br><b>Arch Linux</b></a><br>
      <sub>You will tell people. Often.</sub>
    </td>
  </tr>
</table>

## Not ready to commit?

That's fine, relationships take time.

- **Live USB:** flash the ISO with [Ventoy](https://www.ventoy.net/) or [balenaEtcher](https://etcher.balena.io/)
  and boot it without touching your disk.
- **Dual boot:** keep Windows in the corner for games. We won't judge. Much.
- **WSL2:** WSLg may run the GUI, but we don't test it. If it works, enjoy it. If it breaks, you
  were warned in writing.

Once you're in, head back to [Building](../README.md#building). We'll keep the terminal warm for you.
